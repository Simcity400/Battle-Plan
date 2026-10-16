# dashboard-world-tension

The world tension product: the world tension the game showed over a branch, each country's contribution to it and the events behind it. Behaviour is documented beside the code in [products/world_tension.rs](src/products/world_tension.rs) and in the widget guide [world tension](../../ui/src/widgets/world-tension/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::world_tension`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports it at the old
`dashboard::` paths, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-world-tension` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation, and the tension frames (`world_tension::test_fixtures`) for the dashboard's tests.
