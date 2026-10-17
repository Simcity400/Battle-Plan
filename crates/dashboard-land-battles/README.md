# dashboard-land-battles

The land battle products and the division log: land battle manifests, windows,
rankings, courses and terrain, and each division's combat history and totals.
Their behaviour is documented beside the code in
[products/land_battles.rs](src/products/land_battles.rs) and
[products/division_log.rs](src/products/division_log.rs), and in the widget
guides ([land battles](../../ui/src/widgets/land-battles/README.md),
[division log](../../ui/src/widgets/division-log/README.md),
[shared battles](../../ui/src/widgets/battles/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md)
re-exports them at `dashboard::products::<name>`, registers them, and prepares
and queries them in its pipeline; [dashboard-naval](../dashboard-naval/README.md)
reads the land battles' country filter.
`CENSUS_TABLES` declares every land-census input for the shared walk and direct reads.

The division log's block preparation, for the dashboard's family prepared on
its own, is in [products/division_log/block.rs](src/products/division_log/block.rs):
the block extract a pre-pass worker runs from the block's first hour, its fold
onto the carried scan, and the branch whose block windows, manifest and totals
are written at its next commit.

`cargo test -p dashboard-land-battles` builds and runs only these products'
tests.
