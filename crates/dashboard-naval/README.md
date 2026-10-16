# dashboard-naval

The naval products: naval battle manifests, windows, rankings and details,
naval losses and ships sunk, all built on one retained sinking scan. Their
behaviour is documented beside the code in
[products/naval_battles.rs](src/products/naval_battles.rs),
[products/naval_losses.rs](src/products/naval_losses.rs),
[products/ships_sunk.rs](src/products/ships_sunk.rs) and
[products/sinkings.rs](src/products/sinkings.rs), and in the widget guides
([naval battles](../../ui/src/widgets/naval-battles/README.md),
[naval losses](../../ui/src/widgets/naval-losses/README.md),
[ships sunk](../../ui/src/widgets/ships-sunk/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md) and
[dashboard-land-battles](../dashboard-land-battles/README.md), whose country
filter the naval battles share. The modules keep their dashboard paths,
`products::<name>`, and name what they use at the crates that own it
(dashboard-base, dashboard-text, dashboard-core and dashboard-land-battles). The
[dashboard](../dashboard/README.md) re-exports them at
`dashboard::products::<name>`, registers them, and prepares and queries them in
its pipeline. The crate implements `ReplyWarnings` for its replies.

`cargo test -p dashboard-naval` builds and runs only these products' tests.
