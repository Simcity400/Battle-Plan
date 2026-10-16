# dashboard-raids

The raids product: one observer of every country's raid launches and results,
shared by the raid log and the raid tally. Behaviour is documented beside the
code in [products/raids.rs](src/products/raids.rs) and in the widget guides
([raid log](../../ui/src/widgets/raid-log/README.md), [raid
tally](../../ui/src/widgets/raid-tally/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-raids` builds and runs only this crate's tests. The
`test-support` feature compiles the whole-recording loader, the parity oracle
the dashboard's pipeline tests compare against.
