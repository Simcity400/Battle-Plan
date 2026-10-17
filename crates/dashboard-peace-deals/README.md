# dashboard-peace-deals

The peace deals product: captured peace finalizer actions, with a fallback to dated derived peace outcomes. Behaviour is documented beside the code in [products/peace_deals.rs](src/products/peace_deals.rs) and in the widget guides ([peace deals](../../ui/src/widgets/peace-deals/README.md), [territory transfers](../../ui/src/widgets/territory-transfers/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::peace_deals`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports it at the old
`dashboard::` paths, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-peace-deals` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation, and the peace frames (`peace_deals::test_fixtures`) for the dashboard's tests.

The block extract keeps countries, states, provinces and country counters
reduced to the fields the observer reads. It builds each reduction only when
the source view changes, reusing carried rows with their current supplying
date. Missing and truncated views keep their original evidence.
