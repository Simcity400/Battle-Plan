# dashboard-stability

The stability products: displayed stability, war support and maximum command power over a branch with the breakdown behind them, each part over time, and the daily country scans (`prepared_stability`) that feed their prepared series. Behaviour is documented beside the code in [products/country_stability.rs](src/products/country_stability.rs), [products/stability_parts.rs](src/products/stability_parts.rs) and [prepared_stability.rs](src/prepared_stability.rs) and in the widget guides ([stability](../../ui/src/widgets/stability/README.md), [command power](../../ui/src/widgets/command-power/README.md), [country measure](../../ui/src/widgets/country-measure/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
modules keep their dashboard paths, `products::country_stability`, `products::stability_parts` and the crate-root `prepared_stability`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports them at the old
`dashboard::` paths, registers them, and prepares and queries them in its pipeline.

Preparation projects the scan's resolved parts directly into series. The empty
part dates the supplying read; other parts keep their exact raw value and the
scan's supplying-read character names. Query samplers build their own selected
sample map from the same scan.

The parts observer updates its country naming projection from recorded appearance
row changes. Only name, cosmetic tag and original-tag operands rename parts;
other appearance fields retain that projection. Snapshots, availability changes
and resume rebuild it from the complete table.

`cargo test -p dashboard-stability` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.

Equal authenticated part and measure values carry their numeric runs without
materializing endpoints on each read. Supplying-read events still retain their
own dates, and starts, boundaries, unknowns and holder replacements take ordinary
observation. State reads after observation inspect a materialized current key;
a carried value uses its authenticated continuity proof instead.

Displayed stability, war support and command-power arithmetic is shared through replay-data's `country_stability`; the product re-exports it as `formula`. The occupation hover uses the same stability calculation.

Experience publishes its ten channels when a held read starts or changes.
Gain changes still update the scan's complete daily intervals; extending the
same held read carries its channels through the accepted frontier without
repeating actor-key construction or holder publication.

A parts breakdown locates its country's contiguous line range before building
its ten modifier groups. Other countries' lines are never scanned for that
breakdown. Dirty country selection, supplying-read character names and every
part's dated holder events use the same scan; phase labels distinguish country
breakdown construction from part publication.
