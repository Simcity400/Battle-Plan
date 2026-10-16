# dashboard-occupation

The occupied states and occupation trend products, in one crate because each names the other: occupied states as recorded in prepared replay-block windows, and occupier means from the same preparation. Behaviour is documented beside the code in [products/occupied_states.rs](src/products/occupied_states.rs) and [products/occupation_trend.rs](src/products/occupation_trend.rs) and in the widget guides ([occupied states](../../ui/src/widgets/occupied-states/README.md), [occupation trend](../../ui/src/widgets/occupation-trend/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
modules keep their dashboard paths, `products::occupied_states`, `products::occupation_trend`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports them at the old
`dashboard::` paths, registers them, and prepares and queries them in its pipeline.

`cargo test -p dashboard-occupation` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.
