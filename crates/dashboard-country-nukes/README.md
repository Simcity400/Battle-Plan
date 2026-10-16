# dashboard-country-nukes

The country nukes product: every country's nuclear stockpile as the game's top bar shows it. Behaviour is documented beside the code in [products/country_nukes.rs](src/products/country_nukes.rs) and in the widget guide [nuclear arsenal](../../ui/src/widgets/nuclear-arsenal/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::country_nukes`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports it at the old
`dashboard::` paths, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-country-nukes` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.
