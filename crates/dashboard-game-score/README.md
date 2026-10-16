# dashboard-game-score

The game score product: end-screen country scores at each acquisition read, never cumulative totals. Behaviour is documented beside the code in [products/game_score.rs](src/products/game_score.rs) and in the widget guide [game score](../../ui/src/widgets/game-score/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::game_score`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
It depends on [dashboard-wars](../dashboard-wars/README.md) for the casualty rows it shares with war casualties.
The [dashboard](../dashboard/README.md) re-exports it at the old
`dashboard::` paths, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-game-score` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.
