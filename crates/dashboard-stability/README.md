# dashboard-stability

The stability products: displayed stability, war support and maximum command power over a branch with the breakdown behind them, each part over time, and the daily country scans (`prepared_stability`) that feed their prepared series. Behaviour is documented beside the code in [products/country_stability.rs](src/products/country_stability.rs), [products/stability_parts.rs](src/products/stability_parts.rs) and [prepared_stability.rs](src/prepared_stability.rs) and in the widget guides ([stability](../../ui/src/widgets/stability/README.md), [command power](../../ui/src/widgets/command-power/README.md), [country measure](../../ui/src/widgets/country-measure/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
modules keep their dashboard paths, `products::country_stability`, `products::stability_parts` and the crate-root `prepared_stability`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports them at the old
`dashboard::` paths, registers them, and prepares and queries them in its pipeline.

The parts observer updates its country naming projection from recorded appearance
row changes. Only name, cosmetic tag and original-tag operands rename parts;
other appearance fields retain that projection. Snapshots, availability changes
and resume rebuild it from the complete table.

`cargo test -p dashboard-stability` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.
