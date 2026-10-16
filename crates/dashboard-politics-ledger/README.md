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
and prepares and queries it in its pipeline. The crate implements `ReplyWarnings`
for its own reply.

`cargo test -p dashboard-politics-ledger` builds and runs only this product's
tests. The `test-support` feature compiles its whole-recording test loader and
instrumentation for the dashboard's pipeline tests.
