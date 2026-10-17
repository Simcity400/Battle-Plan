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

Unprepared state windows and reached occupier means share one proved look-back observation. Campaign geometry samples each block at its first fresh resistance read, marks those spans approximate and keeps branch-wide identity metadata preparing. Both kinds of observation use the shared content-bound cache; re-asks reuse their blocks and windows.

An unproved window is an ordinary empty window with the shared prefix extent and
pending hours. It supplies no occupation observation until a local proof or
forward preparation covers it.

Look-back read-ahead is bounded by half the available hardware threads, at least one, regardless of the proof length.

Occupation look-backs read immutable shared section updates and register their current block with replay. Their selected reconstruction and proof rules are unchanged; section retention follows measured table sizes and live reader progress.
