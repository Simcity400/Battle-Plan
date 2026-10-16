# dashboard-forces

The force-size products: army size, supply health, air force size and navy
size by country, and the parts-race families air and navy share to prepare and
query their series. Their behaviour is documented beside the code in
[products/army_size.rs](src/products/army_size.rs),
[products/supply_health.rs](src/products/supply_health.rs),
[products/air_force.rs](src/products/air_force.rs),
[products/navy.rs](src/products/navy.rs) and
[products/parts_race_prepared.rs](src/products/parts_race_prepared.rs), and in
the widget guides ([army size](../../ui/src/widgets/army-size/README.md),
[supply health](../../ui/src/widgets/supply-health/README.md),
[air force size](../../ui/src/widgets/air-force-size/README.md),
[navy size](../../ui/src/widgets/navy-size/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md) re-exports them at
`dashboard::products::<name>`, registers them, and prepares and queries them in
its pipeline. The crate implements `ReplyWarnings` for its four replies.

`cargo test -p dashboard-forces` builds and runs only these products' tests.
The `test-support` feature compiles the army's synthetic frame and the force
families' synthetic recording and prepared store for the dashboard's pipeline
and query tests.
