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

Every captured country's interval extends to each hour it is read, so no hour
is quiet for the observer. An hour joined from its changed rows at which the
captures only extend, and which selects no country (no census, link, capture
identity, power, government, allocation, completeness, daily gain or naming
change, no capture's expiry, no day boundary of a country's open day), takes
the observer's advance path: each extended capture's last hour and expiry move
in place, the expiries of every capture due at the hour together, and the
accounting does what it does when it selects no country (the branch's deferred
warning counts, the census run, modifier formats, hold names after a resume,
the read kept for the next hour, the census series, publication pruning, native
day settlement and the captured parts), visiting no country. Every other hour
takes the whole accounting. Both publish and keep the same bytes.

A block read alone is joined on its own (`prepared::block_extract`, on the
join's tables `prepared::BLOCK_TABLES`): its facts (`prepared::BlockFacts`)
are the join the block's first hour leaves and each later hour at which the
hour's changed captures only extend to it while nothing else joins a country.
A join's state after an hour follows from its state before and the hour's
tables and changes alone, whatever rows the observer has recorded. Observing a
block with its facts (`Prepared::observe_folded`), once the join the first
hour leaves is the facts' own, such an hour that also takes the advance path
for the observer (no row it has not recorded is observed again, no linked
capture is due once the captures move, no day boundary, nothing dirty) takes
it without visiting its captures: their expiries move at the hour, and their
witnesses reach the last hour they extended to before any other join reads
them (`Prepared::end_block` after the block). Every other hour is observed.

An hour presenting many countries presents them on several threads, in
windows, each as presenting it in turn makes it: a country's presentation
reads its own rows, holds and coverage and what every country shares (the
branch's warnings, names, census runs, modifier definitions and read
extent), and taking an earlier country clears only that country's mutations
and governments. It does so only once the content's power diagnostics are
reachable (`PoliticsContent::power_requested`), since a presentation reports
them only from the first item that demanded the power content.

Daily gain and cost publications join from their changed owners when the
recorded deltas are available: those owners' daily keys, applications, wrapping
cost sums and reconciliation counts are replaced, and other owners retain
their indexes and deferred coverage. Recorded cost keys are pruned again only
after a nonempty cost delta or a read whose rows may all have changed; an equal
cost acquisition keeps the previous prune. Rebuilt tables, changes to census keys,
links or the dictionary, and incomplete reads whose former rows must be
removed use the whole-table join. Each country's presented payments share
indexes of their source identities, item definitions, categories and row
order, and select a tooltip's effects once. A balance sentence reuses its
spending item's resolution before tooltip and upkeep grouping changes it;
all response-budget checks still run in their original order. Series keys look
up borrowed tag and field operands without encoding them; publication and saved
state encode the same sorted flat key map.

`cargo test -p dashboard-politics-ledger` builds and runs only this product's
tests. The `test-support` feature compiles its whole-recording test loader and
instrumentation for the dashboard's pipeline tests.
