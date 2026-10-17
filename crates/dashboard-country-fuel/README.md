# dashboard-country-fuel

The country fuel products: each country's fuel stock and capacity, the awards
behind each change, consumption by use, and fuel demand with its history and
stack. Their behaviour is documented beside the code in
[products/country_fuel.rs](src/products/country_fuel.rs) and in the
[fuel widget guide](../../ui/src/widgets/fuel/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::country_fuel`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)). The [dashboard](../dashboard/README.md)
re-exports the module at `dashboard::products::country_fuel`, registers it, and
prepares and queries it in its pipeline.

The country-key join uses recorded row field masks to retain its keys through
balance-only country changes. Country key edits, link changes and snapshots
rebuild or compare the ordinary join; runtime joins rebuild after restore.

Progressive fuel levels and capacities share compact block observations across
summary and state requests. Moving prefixes clip retained keyframes or exact
block folds; completed-day accounting still comes only from prepared pages.

The daily block extract assembles each supplying read's demand lists before
retaining it. Its fold passes those exact rows to the ordinary observer, including
the join's reasons for incomplete parts. Retained reads hold no FuelDemandParts
view, so later part deltas can update the decoder's baseline in place.

`cargo test -p dashboard-country-fuel` builds and runs only these products'
tests. The `test-support` feature compiles their test oracles and instrumentation
for the dashboard's tests.

Fuel day settlement runs on changed supplying observations and each pending
read's 23-hour decision deadline. Between those boundaries its already computed
integer day values hold constant. Publication extends the accepted span with
one ordinary constant run, preserving the supplying strip point and the exact
dated decision event. No repeated-hour rounding or incremental sum is involved.
The next decision deadline is transient and rebuilds after restore. The first
accepted cutoff after publication materializes carried day rows, charging the
same new batch rows as hourly settlement; later equal extensions add no bytes.
