# dashboard-naval-yards

Prepared repair queues and active refit work, above dashboard-base and
`dashboard-keyed`. The two products each return one country-selected keyed
window. One management history projection shares the ship/model/order joins
and content-independent naming for both views, reusing dashboard-forces' typed
`naval_management::projection::{hull,refit}`. The source's ordinary observation
LRU coalesces the projection between the two families, retaining completed
blocks' changed projections together; each keeps its own supplier coverage,
series and continuation. Neither query decodes campaign
frames or parses management prose. The history adapter lives here, not in
`naval_management`: that product is a one-frame drill with no prepared history
to share, and its typed per-frame operands are what both views reuse.

## Public registration

- `products::naval_repairs::{NavalRepairs, RepairsObserver, FAMILY, query}`:
  product/store `naval-repairs`; primary read `NavalRepairQueue`.
- `products::naval_refits::{NavalRefits, RefitsObserver, FAMILY, query}`:
  product/store `naval-refits`; primary read `NavalRefitProgress`.
- `Product::types()` includes every nested wire declaration, shared
  `NavalYardsParams`, `YardGroup`, `YardChoice` and keyed-window types.

The lead registers both observers as keyed independent families, their gates,
query routes/caches and Product metadata, then regenerates frontend types.
The workspace uses `crates/*`; no root manifest edit is needed for membership.
The dashboard still needs its dependency/re-export/registry entries.

## Reply and names

Params: `{branchId, tag, firstHour?, lastHour?}`.
Repairs: `{window, ports:[{country,id,name,ships:[{id,name}]}], tokens, warnings}`.
Refits: `{window, lines:[{country,id,name,ships:[{id,name}]}], tokens, warnings}`.
Group IDs are decimal province/full-line handles; ship IDs are full decimal
handles. `country` is `TAG@slot`, never a merged tag. Groups catalogue the
selected accepted window, not moment membership; the window's presence runs
date membership, queue order, names and all numbers. Names on groups are the
last known names in this requested window; playback uses the dated token
series. `tokens` maps each demanded interned integer to `{id,name}`; `id` is
the raw content key/reference. Missing names are labelled unavailable and
named/count-warned; they never hide numeric values. No unselected tokens are
returned. The caller attaches the family's prepared coverage/pending extent.
Before publication the same object has an empty window and groups.

Names (parts separated by `|`; integers travel as exact decimal strings):

- `TAG@slot|<province>|present,name,state,priority,policy,dockyards,queueCount`
  (one name per measure). `name` and `policy` hold interned token IDs.
- `TAG@slot|<province>|queue|<ordinal>` holds a full ship handle: queue
  order and repeated references remain exact.
- `TAG@slot|<province>|ship|<ship>|<measure>`: `present`, `hull`, `name`, `nameRebuilt`,
  `maximum`, `class`, `design`, `designName`, `task`, `taskName`, `repairMode`,
  `policy`, `repairSplit`, `repairTarget`, `rejoin`, `rejoinResolves`,
  `refitIntent`, `refitIntentName`, `afterRepairIntent`,
  `afterRepairIntentName`, `child:<ordinal>`, `childResolves:<ordinal>`.
- Refits use `TAG@slot|<line>|live,progress,cost,dockyards,target,targetName,
  remainingCostRaw`. Ship membership/measures replace province with line.
  Only hourly progress certifies live membership; daily metadata can remain
  independently readable when liveness is unknown, without active ship rows.
- `TAG@slot|total|<measure>`: repairs `usedDockyards`, `repairLimit`,
  `buildingRepairLimit`, `dockyardsRaw`, `dockyards`, `queueEntries`,
  `distinctQueuedReferences`, `distinctQueuedShips`, `queueCensusComplete`;
  refits `censusComplete`, `ships`, `lines`, `dockyards`, `remainingCostRaw`.
- `TAG@slot|known|<TableId>` holds 0/1, `...|read|<TableId>` holds its
  independent supplying hour for daily siblings where known. Hourly
  suppliers use the window reads and their own known gate. `...|refusal|<interned id>` holds
  that observation's affected count; query warnings sum selected observations.

The global window `known` proves the country identity tables, not every
supplier. Always consult the supplier-local `known` channel before treating a
missing row as absent. Missing a field is not numeric zero. Hull, cost,
progress and raw dockyard totals are fixed5 (100,000); ordinary dockyard
counts and allocation/priorities are unscaled. No raw amount is clamped.
A hull fraction needs a usable positive maximum; refit fractions use hourly
progress against separately dated positive finite last-recorded cost.

## Semantics and preparation

Port ownership comes from NavalBases, not geography/controller. Queues need
a complete queue and ownership census before national zero/counts are proved.
`distinctQueuedReferences` deduplicates full queue handles across owned ports;
`distinctQueuedShips` additionally requires every handle to resolve against
complete Ships. These are observed wait state, never starts/completions.
CountryFactories' naval total (truncated fixed5) is the dockyard denominator;
the building-repair factory limit remains a different field.

Refit links resolve through the complete hourly progress census, never the
daily production roster. Stale/generation-reused lines produce no active
ship rows. Shared lines contribute dockyards/cost once. Missing cost/allocation
omits the national total rather than assuming zero; remaining cost requires
every active line's usable positive finite cost and finite signed progress.
Standing orders are tooltip operands, not queued refits. Rejoin/child references
are retained even when stale; a missing TaskForces census never proves false.

Both FAMILY entries use field-driven extraction over the same selected-table
block pre-pass; sailing, organisation and other unread fields stay quiet.
Quiet spans retain the ordinary keyed hold step. The management adapter
forwards independently witnessed sibling reads from those spans to the same
keyed extract, advancing equal-value daily dates without rebuilding the ship
projection. A newer sibling read is never replaced by an older carried table
stamp. Repeated ships share one hull/model/order projection across repair ports
and refit lines. Read channels and token IDs remain local to each published
view. Recorded block bounds and gaps certify quiet steps; a quiet block endpoint
stamps the carried state for continuation. Recording gaps, unreadable blocks
and unavailable suppliers clear their supplying dates. No independent campaign
scans or per-ship seeks occur in queries, and no preceding daily read is guessed.

Naming uses recorded lexical class spellings, shared variant naming,
recorded-language task-force text and the content catalogue's localisation.
Port overrides and owner-qualified victory-point keys fall back to shared
state-name presentation. Numbers survive missing content. The general Ships
census supplies each ship's independent retained cache and saved naming packet.
A readable cache (including empty) takes precedence. Otherwise `nameRebuilt`
retains naming operands and the recorded naming variant's group (the selected
pool variant only when the naming variant is null). Queries rebuild through the
recorded stack's shared name database with `resolve_any`; authored prefixes,
ordered/unordered entries and fallbacks are the database's own rules. No order
is allocated and legacy generation stays a named counted refusal. Programmatic
overrides defer to a known nonnull group; nonprogrammatic nonempty overrides do
not require a group. Raw cached/override text keeps the recorded-language policy.
Rebuilt text is not proof of cache freshness, and variant group metadata keeps
its separately dated supplying read.
No rate, ETA, inferred completion event, queued refit or artwork is fabricated.
No image requests are introduced; design art remains the existing equipment
route for the lead to request on demand.

Tests: `cargo nextest run -p dashboard-naval-yards`.
Synthetic tests cover queue repeats/order, independent hull/metadata, counts,
full-reference liveness/rejoin, daily dates/equal reads, gaps, continuation,
shared projection work, field-driven quiet spans and equal sibling reads,
append/prefix clipping, cold queries, cancellation, reply budget, layered/mod
names and 5,000 countries/ships. No game data is written or checked in.

Ship name tokens are selected like other labels: `...|ship|<handle>|name`
contains the recorded-cache token, or `...|ship|<handle>|nameRebuilt` the
rebuilding operand token. Only demanded tokens resolve, through the shared
stack-bound database; named/countable failures preserve all numeric runs.
Group `ships[].name` is the last known name in the selected window; the dated
series is authoritative for playback. Renames, gaps and full-reference reuse
never use combat/pride snapshots or spelling as identity. Product/Observer
registration and the reply wire types are unchanged.
