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

`cargo test -p dashboard-politics-ledger` builds and runs only this product's
tests. The `test-support` feature compiles its whole-recording test loader and
instrumentation for the dashboard's pipeline tests.
