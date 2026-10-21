# dashboard-base

The variant catalogue retains equal unavailable module/upgrade operands without
walking every design. Recorded invalidations still select changed designs and
used spellings. Production-line span compaction preserves frozen design
availability and supplying dates, including loss and recovery between line reads.
The shared catalogue and compaction implementation here includes their current
main performance changes so these contracts can be checked on the stacked branch.

Equipment naming observes recorded design mutations between daily censuses in
ordinary and prepared products. The on-demand design sheet joins modules,
upgrades, equipment content, applied company bonuses and organisation snapshots.
It resolves shared named-source snapshots, preserves native residuals and reports
when current source components differ from the design's last cached rebuild.
Build/resource operands, manpower, terrain attrition and mission caches remain
available separately from the calculated stat components.

Country-cost queries join demanded effective operands through recorded modifier
registrations. Individual module XP contributions resolve authored conversion
and XP rules from the verified content stack, with explicit failures for missing
country scope or content. The selected source/target generic-upgrade query keeps mixed authored cost
currencies and the separate generic rounding path. Selected-pair generic and module save-XP queries retain licensing, source-family DLC gates, source/target template selection, target-declared slots, organisation changes and final factor/minimum order. Organisation costs join completed traits, organisation availability/visibility, the initial trait, selected policy and its owning country; unproven inherited conditions/modifiers refuse by name.

Industrial queries compute only the requested source and target. Build,
conversion and dismantle pools retain their separate IC factors, hull costs,
positive upgrade deltas and ordered resources. Resource floors precede
first-smallest pruning and signed integer truncation, then target resource order
is restored. The comparison uses no organisation; an explicit organisation joins
current equipment-filtered trait and policy production bonuses. The same_as_mio
predicate joins the organisation's inherited equipment filter; missing reached
operands refuse individually. The existing detail sections show total IC,
contributors and resources.

Production lines includes an Equipment designs section over the complete saved
roster. Its interactive detail product shows ancestry/designer attribution, all
stat slots, contributors, mission terms, build resources, country operands and
source-specific XP contributions. Mission queries join the selected design's
source terms, native exclusion masks and saved result caches; a contributor
mismatch remains a named refusal without replacing the known native result.
Build resources join recorded resource definitions and token names. Module country admission uses the shared
recorded trigger evaluator. Ship/wing context joins only the selected unit's XP
and assigned ace. Detail reveal shares replay's directed access and reserved
intel rules; the query accepts explicit observer, target and fog context. The default reveal query joins the observed game fog singleton, while explicit fog choices remain conditional queries. Slot required flags and category declarations follow content aliases and inheritance. Full structural dependency/count/role validity and technology/project unlock sources derive from recorded content and country state. Selected wing detail shows the replacement byte and complete ordered equipment bank. Reached missing operands remain named refusals without discarding readable sheet sections.
Faction details show held goals/manifest/sections, current authored conditions and reward descriptions, exact member balances/gains/baselines/upgrades, projection/shares/rank, variables/seed, research inputs, doctrine folders, minimum upgrades, projection totals and faction/system extracted resources. Shared programmes retain their explicit supplying read hour. Advisor and upgrade modifier ownership, research support levels, shared doctrine progress, score explanations and goal command gates join their recorded operands and content. Individually missing condition or dynamic progress inputs are named and counted; independent sections survive sibling failures.

The dashboard's shared product layer, below every product crate and the
[dashboard](../dashboard/README.md) itself: recording sources and their content,
the catalogue loader, the prepared store's shared pages, the product contract and
the shared products other products build on. Nothing here names the product
registry, the preparation pipeline or the service; the dashboard re-exports every
module at its old `dashboard::` path, and product crates name what they use
here as `dashboard_base::â€¦`.

The interactive `faction-details` product selects one full faction handle at one recorded hour. It reads manifests, runtime goals and their retained progress, histories, extra slots and dates through the replay row API. Goal names and descriptions use the recorded content catalogue and faction/selected-country localisation environment. Research/manpower access unlocks, rules, shared programmes and signed manpower contributions are independent sections of the same selected-faction reply. Programme details join existing project tables at the programme table's certified supplying hour, which the reply dates independently of the current reference list; the manpower total follows the native signed wrapping sum. Unavailable tables, content and individual text remain explicit refusals without hiding independent known values.

| Module | What it does |
|---|---|
| [source.rs](src/source.rs), [content.rs](src/content.rs), [catalogue.rs](src/catalogue.rs) | One recording revision with its metadata, resolved content and typed per-source caches; content verification and cache keys; one content catalogue per source and folders (the catalogue itself is in [dashboard-text](../dashboard-text/README.md)). |
| [prepared/](src/prepared/mod.rs) | The prepared store of one recording path and its source key, each branch's published coverage (with the gates the dashboard installs for families prepared on their own), binary series and event batches (with the recorded accumulator calls block folds make again, and the patched family catalogues), frame changes, the contract of a family prepared from blocks (its fields, block extract and fold outcome), exact group sums, progressive (prepared plus recorded tail) query helpers and the per-source cache of their decoded reads, and replies answered from a gate's prepared prefix with the requested hours after it pending ([prepared/prefix.rs](src/prepared/prefix.rs)); the scheduling priority every preparation thread takes from its job (`priority`, re-exported from [job-priority](../job-priority/README.md)). |
| [prepared_chart.rs](src/prepared_chart.rs), [prepared_measure.rs](src/prepared_measure.rs), [prepared_production.rs](src/prepared_production.rs) | Aligned width summaries and event clusters; exact balance and holder observations; production observations and selections. |
| [prepared_map.rs](src/prepared_map.rs) | Sparse native map checkpoints prepared beside the products, and the reader that serves viewer playback from them. |
| [ranking.rs](src/ranking.rs) | Event-based top-N selection and clipping. |
| [read_runs.rs](src/read_runs.rs) | Read-derived value runs and known coverage. |
| [research_dto.rs](src/research_dto.rs) | The research products' reply types, shared with the frontend. |
| [products/product.rs](src/products/product.rs) | The `Product` trait, `ReplyWarnings` and the exported `reply_warnings!` macro, the shared parameter types and validators, and the reply budget check. |
| [products/](src/products/mod.rs) | The shared products: country groups and history, country measure, experience and factories, prepared dockyard repair rates, sampled per-design/residual splits, dated raw design catalogues and per-tag integral/origin reads, equipment, equipment icons, produced equipment and production spending, faction totals, force census, territory and its breakdown, politics content, idea history and format, character portraits and production lines; and the small shared modules several product families name: artwork (focus, technology and equipment art with its research and resource titles), research, resources, place names, country styles, battles and fixed-point runs. |
| [test_fixtures.rs](src/test_fixtures.rs) | Test sources and content options over dashboard-core's recording builders. |

Sprite artwork (`products::artwork::sprites::SpriteArtwork`, key `sprite-artwork`)
draws frames of exact authored sprite keys a product returned, such as doctrine
icons or one mission's frame of a mission strip: `{sprites:[{key, frame}]}` (frame
zero-based) gives `{items:[{key, frame, icon}], warnings}`, each icon a PNG URL or
null with a named, counted refusal. Frames past the authored strip keep their
native offsets: the resolved effect and sampler decide the sample, without a
frame clamp or bounds refusal. It uses the recorded content stack, the shared
sprite renderer and texture decode settings, and infers no suffix,
localisation or recorded hour. `icon_key` checks a key without decoding the
image, so a product can return keys and the widget loads only the art it draws.
Focus and technology artwork keep their own kinds: those read whole content
databases, which an exact key does not need.

## Prepared series

The prepared map's native selection includes naval repair task settings, fleet home-base
automation and intended composition for owned naval hovers. These rows are map inputs;
they do not extend the shared authored-trigger kit used by unrelated products.

Independent family folds share the publication predicate in `prepared::pages`:
a continuing, unmeasured batch below its recording limit waits while pending
hourly row charges plus held pages remain below 64 MiB. Runtime caches are
separate from that publication charge.

Block extracts share supplying-read flags for the same hour and timed tables;
the recording's cadence groups decide them, with CountryIntel dated only by
its own timing. After reconstruction, families sizing the same block share
the sizes of immutable table views. Each family's allocation counts and byte
total stay independent, and the size lookup ends with that block's sizing.
Reduced country views compare only their id, slot and tag before allocating
new rows. A reduction of an unchanged source view reuses its projected rows
with the current sample hour.
Idea extracts also reuse reductions when the block reader reports no change
to that table's projected fields, keys or availability, even when another
field rebuilt the source rows.

A read decodes a key's whole blob in each batch it touches; batches are not
split within a key. `series::decoded_event_rows` reports the event rows this
thread has decoded, so a family that bounds a query by the rows it decodes
(splitting a long event history across keys, as the market ledger does) can
assert it.

Each batch reuses the compression of byte-identical per-key payloads within
that batch. Keys keep separate directories, offsets and payload copies, so
the batch bytes and saved continuation are unchanged by reuse. The temporary
lookup holds each distinct payload's BLAKE3 digest and raw length and refers
to compressed bytes already in the batch; raw buffers are released after
each key.

Change-only projections publish at most once per key and accepted hour. Equal
authenticated values retain their active run without materializing its endpoint;
unknowns, starts and supplying boundaries go through ordinary observation. Direct
state readers seal those endpoints before inspecting them.

Accumulator flushes merge their changed rows with the existing key-state map
in key order, retaining that map's entries and borrowing the pending rows.
Unchanged keys allocate neither replacement states nor pending-row holders.
Inactive keys with no pending rows or partial summaries reuse the empty payload
directly. Every key keeps its
ordinary directory entry and byte extent; active runs and pending summaries are
encoded for the current batch bounds.

Only a bucket with ordered endpoints and the same exact absolute total as its
constant span is eligible for span compaction. Other merged buckets retain their
whole values, including backdated endpoints and sparse totals whose hypothetical
constant span would overflow.

Equal authenticated observations extend their held run in one step, keeping
supplying boundaries, repeated-hour observations and unknown intervals separate.
Disjoint runs in hour order merge summary buckets directly into an ordered
buffer; overlapping or backdated runs use the ordinary ordered map. Event
summary counts group adjacent event hours without a separate map. Payload reuse
compares constant partial summaries in their compact span form before constructing buckets.
Both paths write the same batch and continuation bytes.

A key with no new runs or events can reuse the preceding flush's compressed
payload and next partial summaries. Its identity includes the incoming summaries,
canonical open run, decimal scale and, for every level, whether the current end
adds a prefix break or closes its partial. The first hour cannot change an absent
numeric bucket. Only distinct inputs used by the latest flush survive; restore
starts empty. Directories and separate payload extents remain in every page.

Continuation serialization retains each inactive series key's compact JSON with
the state that produced it. Every save compares the complete current state,
including the bit patterns of summary values, and formats only changed inactive
keys. Active keys, whose endpoints move at publication, stream directly without
retaining another copy. Public state mutations therefore need no separate
invalidation proof. The transient cache is
sized by the current key set, and restore starts empty. The complete continuation
keeps its ordinary JSON field order and compression; neither pages nor saved
layouts change. Directory writes and copying the required output still visit
every published key.

Numeric keys with equal runs, incoming summaries and canonical open run also
reuse summary construction: their input digest finds the encoded payload and
next partial summaries already produced in that batch. Event keys build their
payloads individually. Per-key counters remain in their own directories.

Prepared rank positions use the top-membership sweep. Each input run boundary
starts another callback interval, even when ranks stay equal: callers select
the native country instance from that interval's first hour. Intervals publish
in hour order, then rank order, so an unseeded series batch starts at its earliest
ranked hour. Countries without a known value remain absent, and tags break ties
in recorded UTF-8 order.
Country holder joins use recorded country field masks to keep the same keys
through balance changes; snapshots, key edits and link changes compare their
ordinary operands.

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
it in memory (up to 64 MiB of directories per reader). A query's `SummaryCache`
retains those whole directories by immutable page identity, charged at capacity
with its reply. Later publications reuse only directories read by the current
ask; replaced pages search and read their own directories.
`Reader::latest_event_rows` reads many keys' latest events at one hour, finding
the hour's batch once.

A query's whole reply can retain its numeric summary folds
([`series/query.rs`](src/prepared/series/query.rs)). Each fold keeps its actual
bucket intervals and the immutable identities of its supplying pages. On a new
publication, buckets wholly inside the unchanged page prefix and on the same
grid are reused; changed intervals are folded again. A narrower window keeps
its ordinary reads instead of opening unrelated pages for their identities.
Membership masks, sampled
tails and approximate spans are applied to the current reply after that reuse.
Hourly zooms retain the ordinary full-run lookup, including run bounds that can
extend with later pages. Only the current request's folds survive, and their
allocation capacities are charged with its reply to the observation cache.

That same query entry retains decoded series blobs, parsed JSON event rows and
reconstructed catalogue or whole-reply text. Blobs and rows name the immutable
page identity and their key/ordinal; catalogue text names every base and patch
page in order. An appended page leaves earlier entries reusable, while a replaced
page misses even at the same batch ordinal. Only pages used by the current ask
survive its next publication. Their nested allocation capacities are charged to
the existing content-bound, source-owned query LRU; request shapes compete in that
LRU. Each reply still performs its current coverage, membership, run-boundary and
sampled-tail joins, so reuse does not enlarge exact spans or change a first view's
class. Failed and cancelled asks publish no cache entry.

Projected page values also retain their type, supplying page identities and
projection parameters in that entry, with their owned capacity charged to it.
Country identity reads seal a page once, then clip its finished history for
the current family coverage. Unchanged portions keep their shared runs.

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

Accepted prefix coverage measures preparation progress. Backdated observations and
branch-wide type classifications can revise earlier accepted hours. Those products
use `Prefix::revisable` to mark every presented recorded hour approximate until
completion when the data provides no finite revision bound; pending hours and gaps
are separate. `Prefix::answered` removes independently proven windows from pending.
Total-by-type production selections also carry `typeTotals`, sums of known values
for the selected countries before paging, in type order. This supplies a provisional
automatic type without waiting for the whole-history equipment scan.

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

Stateless country measure, stability and fuel queries use `tail_cached` and
`state_tail_cached`. Related measures share a selection of countries and channels:
one keyframe observation per block for wide tails, one exact fold per block for
short windows. Stateless state reads reuse a whole fold already held by a
summary; otherwise they project only requested points and their immediate
predecessors, retaining the point fold per block. Predecessors preserve the
ordinary identity and value break flags. Re-asks clip these immutable facts to their
viewport and moving prefix; blocks covered by the prefix are not sampled.
These cached point samplers have no frame-dependent refusals outside the
requested points. `state_tail_sparse` runs a separate per-hour check before
skipping projection; callers whose refusals require projection use `state_tail`.
Crossing the exact-zoom threshold replaces keyframe approximations with hourly
folds, including the preceding hour's identity/value continuity.
Factory summaries over a range use `tail_all_cached`, and territory censuses use
`tail_all_cached` and `state_tail_all_cached`, for their existing full-frame
projections; factory points read one frame each (below). Their keys
come from each sampled frame, including aliases not yet in the prepared
catalogue; catalogue growth changes neither the observation selection nor its
cache identity. Territory counts and content-bound areas have separate scopes. Area scopes also
distinguish the current unverified-content choice and whether the area inputs
are available; file identities alone do not establish either.
When no whole fold is held, an exact read caches only the requested extent of
an edge block. Errors propagate directly; a failure never retries the read.

A family read at its own cadence (a daily read) whose sampler answers only from
a read it has seen uses `tail_first_read` and `state_tail_recorded`. A short
tail, and each state hour's block, is read exactly after every block intersecting
its supplying-read look-back, including a read period split across short blocks,
so the read before the window carries into its first hours; the look-back is
the family's read cadence, never a chosen count. The cached fold stays exact
even when short blocks split that look-back into more blocks than the ordinary
exact-zoom threshold. A longer tail uses the family's independent first-read extractor: metrics locate
the first fresh supplying read, and only that hour's selected rows are reconstructed.
Exact at its hour, it stands for the block's hours after the prefix, approximate. A block without such a read is not
sampled.

A family whose lanes hold a state of each frame samples the blocks after its
prefix by their keyframes alone
([prepared/query/keyframes.rs](src/prepared/query/keyframes.rs)): each keyframe is
read and decoded on its own, half the hardware threads ahead of the visit, and
visited in hour order with the block's hours after the prefix, without recorded
gaps. The extractor can return no observation, leaving those hours pending; the
hours of those sampled are the reply's approximate spans. An unreadable keyframe
or one inside a gap samples nothing.

Stateless samplers with presentation operands use `tail_projected`: each
block retains its numeric fold together with the operands captured after its
last observation. Hits restore those operands in block order. The extraction
selection, requested block extent and exact or keyframe mode name each entry;
a moving prefix invalidates only its cut block, while crossing the exact zoom
threshold replaces the approximate folds.

Compact observations also live in a shared per-source cache
([prepared/query/observations.rs](src/prepared/query/observations.rs)). Keys contain
the product, branch, optional block bounds, result type and extraction parameters;
the prepared prefix clips observations covering its window. Concurrent requests share an
extraction. Recording revisions start empty; dashboard requests bind the whole
cache to their recording/content key, and products can bind their own scope.
Failures and cancellations are not retained. Each product's LRU budget is its
largest observed entry, including allocation capacities and charged map/Arc
storage, times the live block count. Exact callers instead declare their actual
window working set; parameter variants share that budget. Caller-held values and
in-flight extraction are transient. `usage` exposes retained bytes, bound and
extraction count in test builds. The cache accepts arbitrary immutable block
facts or proved windows, without assuming a decoder, cadence or predecessor.
`get_query` also retains answers assembled from prepared pages, with their
publication identities in the key. `get_entry` retains at most one current
branch-wide answer per request without a block identity. Its publication binding is separate
from the request parameters: a changed binding releases the previous answer,
including its charged capacity. Branch answers share the product's ordinary
bounded LRU with block observations: largest observed entry times live blocks.
Distinct request selections and viewports can evict each other. Superseded
in-flight extractions finish for their callers without replacing the current
entry. A miss can read prepared pages on the prepared lane; recording reads
inside it enforce the ordinary decode boundary, and busy observations refuse
that lane instead of waiting for another extraction.

Subjects keep compact relation/level observations for each keyframe. Banked
experience keeps all three banks and the read hour once per native holder at a
block's first fresh read; measure and country selection only project it. Its
exact state queries cache each whole block's selected observations
with its supplying-read predecessor, shared by all bank measures, and clip them
for each reached hour or moving prefix. Changing a requested hour inside an
already held block therefore needs no new observation.
Held first-read observations also answer long experience tails on the prepared
lane, without scheduling keyframe workers.

The widget's banked summaries draw the exact prepared prefix and name the
remaining recorded hours pending. They read no unprepared blocks, so their
first reply does not wait for a whole-branch first-read survey. Reached values
continue to use their independent supplying-block observations.

Capacity charges use actual vector and string capacities. On a 64-bit target a
banked holder's compact observation has 64 inline bytes plus its fitted native
key; all three bank measures share it. A subject relation has 80 inline bytes
plus its two tags and optional level token. At 1,500 holders with 20â€“40-byte
native keys, the banked observations occupy about 0.12â€“0.15 MiB per block and
80â€“100 MiB for 663 blocks, plus charged cache entries. Subject sizing follows
the live relation count and token lengths. Exact bank folds retain the selected
holders' three banks and read-hour runs, with a bound of their largest charged
block times the largest exact working set requested. Proved occupation windows
and faction reached replies share the live-block budget across their request
variants; retaining several windows prevents head/hover/prefetch re-asks from
evicting each other. A caller can use the same cache for a stockpile block's
immutable read facts, fold/anchor state or proved at-hour reply, with the
selection and end hour in its parameters.

Every source enables one decoded-section cache on its held replay revision.
Preparation's Depot, pre-pass extracts, the hourly walk and independent query
readers share immutable grouped-table updates, including overlapping selections,
prefixes and concurrent requests. Its byte budget is the sum of each table's
largest measured column allocation multiplied by the live consumer block span;
family, pre-pass and query readers register their progress with replay. LRU
retention shrinks as trailing readers advance or finish, and falls to one measured
block's table set when none remain. Future query selections are not known.
There is no fixed byte or country bound for this cache.
Readers reconstruct their own baselines and keep their own selected availability
and supplying hours. Frames retain their hour's shared update cells without deep
copying a cache hit or keeping the other hours alive. `HeldBytes` deduplicates these
cells when sizing extracts and frame-cache entries. Keyframe-only admissions never
supply ordinary table reads. This retention is separate from the query frame cache
below; revisions share neither cache.

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

Sparse exact-hour consumers use `frames::hours`: ascending requested hours share
one decoder per selection in each block and the same block and hour cache as
windows. Only requested outputs are retained; intermediate hours validate their
ordinary baselines and campaign identity.

`progressive::latest_read` finds the latest complete supplying read within a
family's cadence. Recorded read slots locate its supplying hour first; an
unchanged carry reconstructs only that hour. A snapshot or delta without a read
witness needs decoder validation of the carry's rows. Its `read_window` reads
only the intersecting blocks and keeps requested hours in the same 512 MiB frame
cache as `frame_at`. Windows and seeks share hour slots and table joins; each
window advances its own block decoders, with no lock across windows. An added
table reconstructs only the missing tables. Gaps, unavailable tables and an
expired supplying read leave the value unknown.
Owner-sliced families use their observer to prove every hour's owner read and
identity, rather than treating a held keyframe row as a fresh read.

`progressive::without_block_decodes` runs a query with every block decode refused
(a sampled read with blocks to read, an hour the cache does not hold) and says
whether one was; the data service answers prepared queries this way first. Such a
query takes what the cache holds and never waits on another request's read of a
block or hour: it is refused at once. A product that reads recorded blocks itself
calls `progressive::decodes_allowed` before it reads one.

The `test-support` feature compiles the fixtures, test oracles (the `#[cfg(test)]`
whole-recording loaders of prepared-only products) and the test instrumentation
(read and work counters) for the tests of the crates above, so their test builds
behave exactly as this crate's own. Only dev-dependencies enable it.

Territory's one paired-holder traversal publishes controlled, owned and occupied
amounts. Occupied means a resolved controller and a different resolved legal owner
at that hour; null holders contribute none and unresolved non-null references leave
the dependent measure unknown. States use StateRow holders; provinces and signed
runtime victory points use ProvinceRow holders independently. Map area sums the
same exact content land-raster weights over the occupied province set; missing map
operands leave area unknown without erasing the other measures. No diplomatic filter
applies. Occupied regional breakdowns remain invalid parameters. Faction totals
reuse the selected members' occupied values,
including foreign control between members, with their ordinary missing-member gates.

`TerritoryRun.values` has twelve columns: controlled states/provinces/VP/area,
owned states/provinces/VP/area, then occupied states/provinces/VP/area. The occupied world
columns total foreign-controlled units independently of the selected countries.
`territory::columns` names wire columns; `territory::prepared::VALUE_CHANNELS` maps
them to numeric/rank channels. Channel 8 stays the scoreboard's native owned-state
counter; occupied states/provinces/VP/area use channels 9/10/11/12. Regional observed inputs
retain their controlled/owned eight-column prefix, with no occupied projection.
All count measures share the existing progressive observation scope, continuation,
rank sweep, summary reads and publication gates; no additional scan is introduced.

## Country groups first view

Country groups sample later keyframes with the preparation observer, carrying
its rosters and dated name operands through each sampled span. Reply histories
include dissolved factions. A faction without a name read keeps its roster and
literal identity but has no display name or name runs; its reason names the
missing read, counted once per faction. Saved prefixes and resumed scans keep
that absence without inventing a name or supplying hour. Group-change names
keep a counted missing-read refusal when their naming hour has no operands.

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

The selected country lookup follows immutable country/link views and consecutive
country deltas that preserve every identity field and the row count. Snapshots,
link changes, gaps, missing field evidence and availability changes resolve the
country again. Every intervening hour updates the proof. Consecutive non-read
hours with equal holder and line, output, organisation and modifier availability
form one carried span. The fold extends each open line's endpoints once through
that span; supplying reads, gaps and availability transitions retain their own
boundaries. Variant names advance through complete carried spans with their
ordinary gap and read behavior.

Prepared production endpoints aggregate country/variant quantities and spending;
they contain no line lifetimes, queue values, organisation history or modifier
runs. Production-line histories read the recording for those operands. The
persisted whole-country line scan supplies them directly when it is current.

Equipment variant naming keeps only the lexical spellings its equipment types,
module slots, modules and upgrades reference, including missing spellings, with
a weak dictionary identity. Unrelated token edits preserve the naming operands;
used-token edits rebuild them. Naming does not retain the decoder's dictionary
baseline.

## Country experience

The country-experience scan keeps complete hourly gains as runs of equal signed
amounts within a native instance. Changed inputs and banked reads visit their
countries; other countries' open runs hold through the scan's frontier. Daily
reads sum the last 24 complete intervals with wide arithmetic. Missing or
ambiguous gains, instance changes and identity gaps break that day. Quiet spans
advance the frontier without visiting countries. The saved runs and recent
banked reads resume with the observer; runtime joins rebuild from the first frame.
Banked and gain row deltas join only their affected slots, including every raw
alias of each slot. Several rows resolving to one slot remain unknown. Rebuilt
tables, link changes and availability transitions use the whole join; banked
rows are read directly from the accepted frame without retaining another copy.

Prepared production spending projects a holder's equipment inventory only when
its units or cost version, identity, continuity or scope refusal changes. Equal
versions retain their active numeric keys without visiting their variants.
Certified quiet spans and unchanged hourly ingests carry current holder cache
proofs through the accepted frontier without projecting their inventories.
Removed holders close their own keys. The runtime projection rebuilds after
restore and reconciles all saved active keys once; rockets without inventory
versions project on every observation. Published values and continuation layouts
use the same keys and series.

Each output row records what its input was for: new production, a ship refit
or a conversion of stockpiled equipment into the target design. Input of a
production tick whose purpose was not recorded is its own entry (`unknown`): a
reply split by purpose keeps it apart, and replies that do not split count it
with new production, as Equipment produced does. That entry also keeps the
cumulative magnitude of such input (`unproven`), which the production family
publishes as its own dated series per tag and design, beside the spent series,
so a range or month reads whether such input joined it against its own
baseline. Designs that ever had converted or unrecorded-purpose input also get
that purpose's series (`purpose_keys`), which holds its amount's endpoints; the
series without refits holds them with new input, so a split reply has no
new-production peak for such a design.

Dockyard spending ([products/dockyard_spending.rs](src/products/dockyard_spending.rs))
and Military factory spending ([products/military_factory_spending.rs](src/products/military_factory_spending.rs))
read Production spending's prepared endpoints split by purpose (one leaf per
design and purpose), keeping their domain's designs (naval; air or land) and
those content cannot resolve. They show the purposes the domain has (new
ships and refits; new and converted) and input whose purpose was not recorded
as its own part. A purpose the domain never has rules an unresolved design
out; on one of the domain's own designs it stays in the whole as an Other
purposes part and is counted in the warnings, never folded into a shown
purpose. Their months views
([products/purpose_split.rs](src/products/purpose_split.rs)) difference
production's shared spent series at calendar-month ends, inside one
observation, from zero before a design's first input or at its observation's
start: new production is the input without refits less its converted and
unrecorded-purpose input, refits the whole input less the input without
refits, converted and unrecorded-purpose input their own series' change, all
against the combined series' baseline; each month also carries the input of
purposes the domain never has. A month is known from the equipment source's
first read until a taint, an observation restart or a change of the
country's holder, as Production spending's coverage, read from the endpoint
page holding the month's last hour. Dockyard
repairs integrate the prepared repair rate over the same hours. Dockyard
spending answers through the hours the production and repair families have both
prepared; Military factory spending through production's.

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
fields. A reduced table is built only when its source view changes; carried
views reuse the reduction with the current supplying date. Country reductions
copy their keys directly without an intermediate key catalogue.
Each hour read is observed as hourly preparation observes it; the
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

Prepared `idea-history` requests name `{branchId, tag}`, optionally `through`,
`preview` and an at-hour `hour`. They answer the family's exact prepared prefix,
including an empty prefix before its first commit. The reply's `pending` hours
exclude recorded gaps. With `preview`, each later keyframe supplies held ideas,
laws and appointed advisors for its block, with `approximate` spans; sampled
edges never claim acquisition or removal dates. The keyframes read ahead through
`keyframes::blocks_after`, with one function sampling one block's keyframe for a
country. Compact holdings and naming use the per-source observation cache, keyed
by tag and selected tables; a moving prefix or shorter preview clips the same
observation without reading, decoding or extracting that block again.

An hour beyond coverage adds `held`: the exact frame's held set, in content
order, with acquisition order, dates and acquisition-context naming marked
`preparing`. The icon grid uses this state; lane widgets use the approximate
geometry and the shared key. Accepted coverage replaces sampled spans with the
observer's exact holdings and dates. The observer retains identity-start naming
for topic replies and span-start naming for event-feed; the reference branch
scan remains available.

## Territory breakdown

A reached regional territory breakdown hour
([products/territory_breakdown.rs](src/products/territory_breakdown.rs)) runs the
regional observer's step over that hour's frame alone, read directly with its
country totals: region series the frame does not reach are absent rather than
zero, and its observed spans name the columns the hour proves. Other regional
views keep their coverage gate.

## Prepared map checkpoints

[prepared_map.rs](src/prepared_map.rs) prepares the map's native inputs and shared
authored-trigger and effect-description tables, including mod-authored hover
effects and their advisor, decision, occupation, operative and scientist scopes.
The same selection supplies preparation and playback: fleet score rosters, ground
crews, daily air rosters, railway-gun transport speeds, reserve modifier trees and
owned war relations are retained alongside the other native map inputs. A dev-only
[source census](src/prepared_map/tests/selection.rs) follows production modules in
every viewer crate and the shared dashboard-text/replay-derive readers, including
typed schemas, viewer table IDs, aliases and macro operands. It names a missing
table and its source function; no consumer exemption or runtime tracking is used.
Independent native checkpoints retain decoded rows,
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

The stored selection is part of the existing progress and checkpoint contract.
Changing it changes no binary layout (`HOIMAP04` stays unchanged): the reader
requires the exact selection, and the producer's metadata-prefix check stops
reusing a prepared store with a different selection. Preparation replaces that
store and rebuilds its products; playback uses ordinary recording reads until
matching prepared coverage is available. No old-cache migration is needed.

The shared [Reader](src/prepared_map/reader.rs) serves viewer playback and
acceptance. It binary-searches checkpoint headers, restores the nearest native
state and calls the ordinary selected recording reader through the target hour.
Source parsing starts at each source block's keyframe; native application skips
the frames preceding the checkpoint cursor. Selected table decoding uses half
the available processors, at least one, with the existing below-normal worker
priority. Consecutive steps mutate the native decoder in place. A step needing a
source load reads the rest of that accepted block once; later steps retain it.
Cancellation walks can apply native state without exporting an endpoint
snapshot, while keeping passed-hour visitors and validation unchanged.
Only one parsed source block remains between requests. Source admission precedes
passed-hour callbacks, and reads never extend beyond accepted branch progress.
No recording format, directory, index or setting is added.
Changed gaps become interval records with reasons in original occurrence order,
including repeats and overlaps. Readers binary-search gap records on disk.

Newest-window proofs can read an earlier prefix by selected key fields
([earlier.rs](src/prepared/query/earlier.rs)). Blocks are change-read in parallel,
then combined from immutable per-block key sets in the shared observation cache.
A longer or shorter prefix reuses retained blocks. Each requested proof's
compressed reply, including an unproven result, uses that cache's content binding
and live-block bound in a separate product scope. No separate history map or
eviction policy is retained. These observations publish no prepared-store layout.

Frame observers use `state_tail` with their supplying predecessor explicitly
selected; a re-ask visits every hour again through the shared frame cache.
`state_tail_recorded` instead caches the sampler's returned block observations,
independently of requested hours and moving coverage. State requests visit every
requested block exactly even when a day is split across many short blocks. The wide keyframe approximation belongs to summary
requests; it cannot omit requested state hours.

Stateless factory state reads and single-hour summaries use `points_cached`:
one reached frame per requested hour, with the numeric observations held in
the shared source cache independently of coverage. They observe no preceding
hour and claim no continuity outside the requested points. Daily carried
values retain their supplying-read folds.

Cold object replies carry an empty preparation extent: `firstHour` is the
branch's first recorded hour, `lastHour` the preceding hour, and `complete`
is false. `prefix::extent` constructs it; `read_or_empty` reads accepted
coverage or uses it, and `mark_unprepared` marks it through the same `Prefix`
writer. Named `preparedCoverage` uses the same extent beside capture evidence.
Keyframe re-asks without block decodes visit retained observations on the
calling thread; missing or busy observations refuse immediately. Air superiority
keeps its viewpoint projections in this shared content-bound cache, charged for
every retained allocation. Supplying-read lookups keep validated supplying hours
in the same bounded observation cache by branch, target hour, table and cadence;
selected table unions are reconstructed through the shared frame cache.

Cold cumulative queries use `prefix::read_or_empty`, the same prefix shape as a
published reply. Production spending has no leaves or numeric totals before its
first endpoint, and its pending total runs carry `preparing`. Production
selection projects an unobserved endpoint through the ordinary selection
builder: there are no country amounts, ranks or known inventory.

Cold research state replies retain the last witnessed slot availability when
current inputs are unavailable. Those slots have no current state or fabricated
read coverage, so the active view shows them as unavailable.
Prepared slot reads retain their supplying hour independently of progress,
including empty slots and reads witnessing a removed slot. Unavailable inputs
keep the preceding supplying hour; a new empty read clears earlier progress.

Prepared map selection retains the Operatives map's mission, operation, agency-buffer, strategic-operative and stability inputs alongside the shared map and trigger operands.

Production lines' newest-block first view and whole-history follow-up share only
their overlapping block extracts in the source's observation LRU. Keys include
the recorded block, country tag, native holder and selected tables. The cache
counts every kept frame allocation and is bounded by the newest window's live
block count and largest observed extract; earlier whole-scan extracts remain
transient. Both folds start from their own first hour, retaining their distinct
line origins, output totals and approximate spans.

Factory and territory rank publication coalesces adjacent spans whose country
and position agree before calling the series accumulators. The rank sweep
still compares every input boundary; only redundant publications disappear.
Factories and army prepare and sort one rank-boundary plan per flush and
reuse it across their channels, borrowing the native runs. Numeric families
own an explicit `series::Payloads` table in each flush and pass it to the
accumulators that share construction. The lookup includes batch bounds,
decimal scale, runs and incoming summaries; ordinary `flush` calls and other
tables share nothing. The table is released when the family's flush returns.

Factory, territory, force-size, trade and construction catalogue owners retain
the last published text and compressed chain between flushes. Reuse checks
all live range identities and the terminating slot in the current writer;
unchanged identities need no payload reads. Unchanged text writes nothing.
A changed catalogue uses the same patch
and compaction rules, reconstructing an older prefix only when merging
patches. Restoration starts this transient cache empty and validates the
published chain once. Its memory follows the current catalogue text and live
compressed ranges; it holds no earlier catalogue versions.

Force censuses use exact country field deltas to carry country joins across
balance-only changes. This requires consecutive hours on the same branch,
unchanged links, the same row count and no deleted or edited identity keys.
Snapshots, gaps and incomplete reads take the ordinary key comparison.

Catalogue writes go through `series::write_catalogue` or a prepared family's
`CatalogueWriter`; both use the same base/patch publication. The cached
families are factories, army, air/navy size, territory, trade, construction
and construction spending. Their lifetimes follow these rules:

- A refusal prevents subsequent flushes and saves; changing the held branch or
  reloading a flushed branch drops its prepared state. A new default state has
  no cached catalogue. Gaps close native runs without changing stored pages.
- Snapshot reconstruction and block extracts change native observations,
  not the published catalogue. Only a family flush writes its catalogue; an
  extractor's local prepared state does not publish one.
- Restoring a continuation creates an empty cache (`serde(skip)`). An append
  restores against the accepted prefix, then the first flush reads the chain.
- Clear cancels and waits for preparation before taking the store lock.
  Content rebinding replaces preparation and drops its branch state. Store
  epochs invalidate restored status; an active cache proves its ranges in the
  actual locked writer. A schema rebuild starts with default branch state and
  a replacement writer. None relies on preserving a catalogue owner's memory.
- Every cache also checks range identities in its actual writer. Another
  prepared instance or a direct range write, replacement, Clear, abandoned
  writes, or pack compaction invalidates changed identities. An unchanged
  append can keep them. The empty/missing terminating slot detects a newly
  appended patch. Writers unable to supply identities disable reuse.
- A fork records identity checks as reads, so another adoption changing a
  checked range refuses the fork's publication. Identity checks open no pack
  and flush no staged payload bytes.

Variant catalogues retain the current definitions and design operands used by
production naming. Changed observations rebuild raw naming only where those
operands differ; used dictionary spellings remain independent of the decoder.
Consecutive country deltas that keep identity fields carry the country-key
projection. Identity invalidation stays pending between supplying reads; weak
view equality cannot clear it. Snapshots, unavailable countries, gaps and
span-only advances require a full key comparison at the next read. Forgotten
names and restoration rebuild the operands.
Production presentation retains counted variant membership by holder inventory
version, replacing only changed holders while dating names at each presentation.
These operand and membership caches are transient.

Reached research replies include the unused grant inventory. The projection preserves repeated IDs and restrictions in native order, proves a continuous supplying owner read at the requested hour and resolves labels through the bound recorded content.

Research numeric state and held grants remain available when content presentation refuses; that refusal supplies a warning, and stale artwork/localisation bindings are cleared.

Faction details include phase-derived account balances, shares, rank and totals; member selections; advisor/spymaster modifier ownership; applied goal-section sources; doctrine-sharing mastery minima; and programme support levels at their explicit daily supplying hour. Goal availability binds leader ROOT and acting-country FROM independently. Command gates combine the runtime DLC mask, membership, authored checks, held goals, category capacity and initiative; term dates remain available without inventing an independent cooldown. Missing reached operands have individual named refusals.

Faction goal confirmation excludes every already-held candidate; each nonzero held status has free replacement cost. Held-goal removal checks do not depend on the replaced goal's own availability. Contribution explanations derive the subject scaler and scientist term from the carried country operand; the expected manpower term uses the controlled-state roster and controller recruitment operands at one explicitly labelled supplying hour, including held available/locked/population words, cores, noncore modifiers, selected upgrade and recorded content defines. A zero effective subject scaler proves zero without reaching those inputs. Only individually missing reached operands refuse a member prediction.
The interactive country-people product answers only the requested reached country and hour. It derives native manpower pool sums, recruitment targets and rates, effective state recruitment and deployed branch manpower, and returns exile resources and election details. Content defines and modifier metadata resolve through the recording's stack; missing operands produce counted named refusals.

Runtime country-people replies and offline content preparation share the validated recruitment defines cache for each resolved stack, including named failures. Missing defines refuse only their dependent recruitment results.

Ordinary army manpower uses native roster/effective/logical ownership and excludes exile-origin templates in ordinary, sent and volunteer loops. Effective recruitment starts with the signed country residual; any missing controlled-state row refuses the total by name while retaining known state details. Ordinary and exile-attributed air crews are independent channels; exile cards show deployed crews even when the wing belongs to the host pool.
Equipment design details show the demanded country's consumable discount roster
and the selected source's clamped discount. Exhausted unpruned entries remain
visible because native selection does not filter remaining uses.

Selected-unit numerical contributors join content static XP modifiers, discrete ship levels, wing tenth-step XP scaling, equipment-pool category exclusions and alive ace definition effects. Own-entry eligibility/clamps and separate owner effectiveness/wing-country bonus stages remain visible. The sub-unit pool uses its recorded database index. Carrier effects join the existing complete positive-equipment flag word, including nonselected hulls. Ship display precision crossing a mod threshold refuses individually; a foreign wing training owner outside existing air-design demands also refuses by name.


Equipment-design detail selects CountryCommandPower with its selected-unit rows, so ace effectiveness and range/speed owner stages remain available in selected replay reads.

Prepared map selection includes division career history and existing manpower contributors for the deployed singleton hover's retained career and exact XP normalization. It shares the recorded operands instead of recording a second normalized XP value.

Research continuations persist simulation history and rebuild allocation budgets from restored allocations on their first observation; process-local capacities are not persisted.

Factory preparation keeps a sparse branch-wide per-observation refusal count. Reached faction totals read that count at the requested hour, so later failures do not appear in earlier replies.
Within one immutable series reader, contiguous event pages retain their merged
frontier and per-key endpoint ordinals. A changed selection, window, cursor or
coverage clamp starts an ordinary seek. Point reads retain each key's last
resolved global run bounds, authenticating the requested hour's local run
before reuse, so a missing hour cannot become known through the retained bounds.
These transient lookups end with the reader.

Stateless resource samples retain their last table-view identities, extraction
selection and numeric projection within a reply. Every visited hour still
passes through the ordinary tail reader and its continuity rules. Country group
histories parse their decompressed JSON as a whole buffer before restoring their
dictionary and roster references.

Country key comparisons in ideas retain an unchanged reduced allocation and use
recorded identity field masks for consecutive deltas. Snapshots and changed
identities compare the complete key projection.

Research coverage, slot availability, state and progress, and territory's country
and regional fact markers carry equal numeric values through accepted hours.
Starts, unknowns and supplying events retain their ordinary observations;
publication materializes the held endpoints.
Research's allocation charges are runtime state, rebuilt from the restored
collections before their first observation; continuation bytes hold no process's
vector or string capacities.

Ordered runs of one numeric key can be pushed together with one accumulator
state and batch lookup. Their canonical runs, counters and saved bytes match
ordinary ordered pushes. Recorded accumulators retain the ordinary calls.

Variant naming keeps reverse dependencies from spellings, naming links and
canonical slots to their definitions. Recorded row invalidations accumulate
until a supplying read consumes them. A snapshot compares ordered source rows
once to select changed definition and design owners. Each selected design reads
only its own module and upgrade ranges; its naming joins only the slots it uses
and slot zero. Replaced and deleted definitions release their dependency
memberships, dropping a spelling, naming link or slot when its last user leaves.
Design operands remain only for current definitions; restore rebuilds these
indexes from the next complete read. Appearance fields other than the original
naming tag leave this projection unchanged. Gaps, refusals and restore retain
the ordinary full-read and naming-unknown boundaries. These dependency indexes
are runtime state.

Prepared map selection includes CountryTemplateState for the existing garrison/staff sections. Its regression encodes a recording, prepares a sparse checkpoint with the production selection and restores it with the production map Reader.
The production prepared-map selection includes the five shared movement-speed
operand tables alongside division movement and status, so checkpoint seek and
subsequent recorded updates retain the inputs arrival estimates consume.
Country identity history resolves captured scope references only with the occurrence hour and matching native country ID/instance witness, retaining alias and missing-read refusals.

Within one immutable series reader, contiguous event pages retain their merged
frontier and per-key endpoint ordinals. A changed selection, window, cursor or
coverage clamp starts an ordinary seek. Point reads retain each key's last
resolved global run bounds, authenticating the requested hour's local run
before reuse, so a missing hour cannot become known through the retained bounds.
These transient lookups end with the reader.

Faction condition and command-gate explanations format failures only when appending reply reason strings.
Within one immutable series reader, contiguous event pages retain their merged
frontier and per-key endpoint ordinals. A changed selection, window, cursor or
coverage clamp starts an ordinary seek. Point reads retain each key's last
resolved global run bounds, authenticating the requested hour's local run
before reuse, so a missing hour cannot become known through the retained bounds.
These transient lookups end with the reader.

The native map selection also retains naval repair-base allocations and ordered repair queues for the Navy base capacity text.

Prepared map checkpoints retain AirTransferInputs for selected-wing transfer arrows and progress; these inputs are ordinary recorded simulation state.

Prepared map checkpoints retain the complete sparse Navy mine vectors and ordered standing incident reports. The mine extent witness, signed amounts and stored full incident identities survive checkpoint reconstruction unchanged.

Faction details share typed influence, manpower, programme, upgrade-name and doctrine/research projections through `products::faction_details::selectors`. Prepared faction histories use those same authorities without parsing tooltip prose. Goal group artwork uses the catalogue sprite renderer; faction and system resource pools copy the shared `ResourceInfo` icon, retaining independent amounts and counted artwork refusals.

Country-people's sent-unit manpower loops use replay-data's shared military-aid reference fold. It retains repeated live references, null/nonlive skips, first matching sender contributions, exile-template eligibility and signed i32 wrapping independently of dashboard aid-count histories.
