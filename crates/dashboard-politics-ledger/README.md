# dashboard-politics-ledger

The politics ledger product: political and command power balances and spending by
country, with the ideas, advisors, laws and decisions behind each line. Its
behaviour is documented beside the code in
[products/politics_ledger.rs](src/products/politics_ledger.rs) and the widget
guides ([political power spending](../../ui/src/widgets/political-power-spending/README.md),
[command power spending](../../ui/src/widgets/command-power-spending/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
module keeps its dashboard path, `products::politics_ledger`, and names what it
uses at the crates that own it: [dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and
[dashboard-core](../dashboard-core/README.md). The [dashboard](../dashboard/README.md)
re-exports the module at `dashboard::products::politics_ledger`, registers it,
and prepares and queries it in its pipeline, where the ledger is a family
prepared on its own, observed hour by hour on its own thread through the
`Prepared` observer's public calls (see the dashboard's "Prepared politics
ledger"). The crate implements `ReplyWarnings` for its own reply.

Every captured country's interval reaches each hour it is read, so no hour
is quiet for the observer. A join from recorded changes keeps countries
unselected when captures only extend or their start and end both move one
hour, retaining their identity and streams. Both proofs require a unique native
country, census slot and tag; shared tags, shared slots and aliases use ordinary
accounting. A sliding window must already have begun, contain no counted unknown
addition and drop no political daily gain; no retained power row may enter at its new end. New event, daily, cost,
allocation, census, link, completeness and naming changes still select their
owners. A dropped gain, unknown addition, identity change or future start
rechecks the affected country in full.

When the join selects no country, no capture is due and no open day reaches
its boundary, the observer takes its advance path. This keeps deferred warning
counts, census runs, modifier formats, hold names after a resume, the retained
read, series, pruning, native-day settlement and captured parts. It visits no
country in accounting or presentation. Other hours use the same accounting
and presentation, selecting only their affected countries and rows.

A block read alone is joined on its own (`prepared::block_extract`, on
`prepared::BLOCK_TABLES`). Its `prepared::BlockFacts` hold the first hour's
join and consecutive spans of equal fixed-start captures or equal sliding
window widths and membership. When the fold's first join matches the worker's,
it can accept a certified hour without visiting its captures, after checking
its own pending rows, diagnostic binding, dirtiness, naming and day boundaries.
The next linked deadline outside the certified captures is checked once per
span and still stops the advance. Expiry sets move whole between hours; sliding
starts and former last hours, whose boundaries have already taken effect,
are removed. Witnesses reach the final certified hour once, before another
join or at `Prepared::end_block`. Mixed fixed and sliding captures take the
ordinary join. Both paths publish and keep the same bytes.

An hour presenting many countries presents them on several threads, in
windows, each as presenting it in turn makes it: a country's presentation
reads its own rows, holds and coverage and what every country shares (the
branch's warnings, names, census runs, modifier definitions and read
extent), and taking an earlier country clears only that country's mutations
and governments. It does so only once the content's power diagnostics are
reachable (`PoliticsContent::power_requested`), since a presentation reports
them only from the first item that demanded the power content.
The same workers build its payment operands and tooltip groups; the fold
consumes them, including any errors, in the country's original row order.

Accounting selects recorded changed rows, unresolved power and cost rows that
this read observes again, and every row of an identity it rechecks. Retained rows
of the same owner otherwise keep their accounting. Selections retain the owner
join's order, including indexed snapshots; indexed selections scan each selected
owner's positions once, regardless of how many unresolved rows it retains.
Native-day counts, latest power hours, daily-gain membership, unknown-addition
counts and wrapping cost sums
remove each replaced or deleted row and add its replacement. Duplicate gains and zero wrapping sums retain their multiplicities.
Equal retained daily rows keep their supplying dates without rebuilding their
accounting wrappers. A selected country's daily series project again only when
its retained operands, published key membership or native day changes, or an
accepted episode starts. Membership edits invalidate every retained tag of that
native country. Native daily publication retains its field ordinals by tag and
recorded identity, preserving their allocation and publication order. These
projection indexes are transient and rebuild after loading saved state.
Empty command allocator sets extend equal snapshots in place: their only
operands are the recorded total and, without it, the refusal. Named allocators,
changed totals or refusals, gaps and restored states build their snapshots.
Capture identity changes recheck their owner's additions; snapshots rebuild the
same aggregates. Reconciliation and publication pruning retain their owner joins.
The selection is rebuilt from each accepted read and is not saved.

Daily gain and cost publications join from their changed owners when the
recorded deltas are available: changed rows update their daily keys, applications
and wrapping cost sums, and only their owners reconcile again. Other owners keep
their indexes and deferred coverage. Daily key membership follows recorded key
changes, and a counted native-day index supplies the newest daily settlement.
Recorded cost keys are pruned only for owners whose occurrence identities may
have changed; amount and presentation edits keep the previous prune. Unresolved
power and cost rows are replaced through the observed owners' key ranges.
Rebuilt tables, changes to census keys,
links or the dictionary, and incomplete reads whose former rows must be
removed use the whole-table join. Each country's presented payments share
indexes of their source identities, item definitions, categories and row
order, and select a tooltip's effects once. A balance sentence reuses its
spending item's resolution before tooltip and upkeep grouping changes it;
all response-budget checks still run in their original order. Series keys look
up borrowed tag and field operands without encoding them; publication and saved
state encode the same sorted flat key map.

## Fold ownership and order

The observer uses three country identities: native `(country, instance,
epoch)` tuples for event admission and daily applications, canonical slots for
census and command-power joins, and recorded tags for histories and presentation.
The recording's links and tag history join them; none is a permanent partition
of the other two. Several native identities can join one slot or tag, and one
native instance can retain its allocators when its tag changes.

Daily rows, mutation histories, daily costs and coverage belong to a tag's
`Country`. Pending event and cost keys, captured witnesses, wrapping cost sums,
daily applications and deadlines are indexed by native country. Holds are
stored by tag, but each read first collects the affected tags' previous
allocators by native instance, then consumes them in canonical-slot order.
This transfer precedes the new snapshots, including tag reuse and alias joins.

The shared input join and its capture certificates depend on recorded tables.
Event admission, deferred warning rates and pending selection also depend on
what the preceding observer accepted. Event-dated naming and scripted text read
the complete supplying world, including target countries and its faction,
ideology, war and localisation operands. Equal organisation facts retain the
first supplying event's date through the shared fact pool; that date survives
inside unresolved rows' saved facts. Optional content diagnostics become
reachable when the first ledger item requests them, in observation or
presentation order.

Publication uses one accepted-hour axis and one native-day frontier. The newest
native input can close an open day containing another country's payments.
Settlements visit payments, categories, daily rows, raw costs, governments and
government lags in that order before the supplying hour's presentation. Ordinary
presentation visits dirty tags in order, then projects parts in a second pass.
Series ordinals are allocated at first encounter across these phases; sorting
the flat catalogue preserves those ordinals rather than assigning new ones.
Events retain their supplying order within each key. Tail range ordinals are
also retained across publications and resumes. An hour's accounting precedes
its first presentation error; only the successful presentation prefix has
changed the published metadata and cleared its mutations at that point.

`cargo test -p dashboard-politics-ledger` builds and runs only this product's
tests. The `test-support` feature compiles its whole-recording test loader and
instrumentation for the dashboard's pipeline tests.

Spending totals can answer before the first catalogue exists: the empty reply
has no payments or spending coverage. The dashboard marks its requested hours
pending, then replaces it with exact prefix totals as the ledger publishes.
No empty prefix claims zero spending or a recording gap.

The live spending ledger shares that cold reply with spending totals. Both read
only through the accepted prefix and keep following their joint coverage; an
unprepared live hour does not claim an empty day or zero spending.

Certified capture spans retain the next linked deadline outside their advancing
captures. Consecutive hours with the same certificate carry that check and the
expiry bucket membership; other deadlines still stop at their recorded dates.
Ordinary joins discard the deadline proof, and captures with future starts keep
each hour's check. Phase labels separate capture joins, deadline checks, expiry changes and
materializing the witnesses of sliding windows.
Observer phase labels also separate clock advance, input joins and certificate
checks, accounting selection, names, presence, coverage, events, holds and
completion, the retained read, daily key membership and capture checks for
parts. Presentation workers label payment projection; native daily settlement
labels its projection separately.
