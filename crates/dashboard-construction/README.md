# dashboard-construction

The construction log and construction spending products, in one crate because
each names the other: every building and railway project of every country's
construction queue, and the civilian factory output spent on each. Behaviour is
documented beside the code in
[products/construction_log.rs](src/products/construction_log.rs),
[products/construction_spending.rs](src/products/construction_spending.rs) and
in the widget guides ([construction
log](../../ui/src/widgets/construction-log/README.md), [construction
spending](../../ui/src/widgets/construction-spending/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the modules
keep their dashboard paths and name what they use at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports them at their old
`dashboard::products::` paths, registers them, and prepares and queries them in
its pipeline.

`cargo test -p dashboard-construction` builds and runs only this crate's tests.
