# dashboard-air-superiority

The air superiority product: each strategic region's friendly and enemy air
scores, as moment windows and daily timelines. Behaviour is documented beside
the code in [products/air_superiority.rs](src/products/air_superiority.rs) and
in the widget guides ([air superiority
region](../../ui/src/widgets/air-superiority-region/README.md), [air superiority
timeline](../../ui/src/widgets/air-superiority-timeline/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-air-superiority` builds and runs only this crate's
tests.
