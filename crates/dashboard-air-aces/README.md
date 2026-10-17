# dashboard-air-aces

The air aces product: every country's ace roster, dated by successful reads,
with each ace's wing, aircraft and death, and the ace portraits. Behaviour is
documented beside the code in [products/air_aces.rs](src/products/air_aces.rs)
and in the widget guides ([aces](../../ui/src/widgets/aces/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

Ace preparation restores active roster baselines from the shared resume page
when it contains the ace table at the saved hour. Otherwise it seeks that hour's
campaign and ace tables from the recording before continuing.

`cargo test -p dashboard-air-aces` builds and runs only this crate's tests.
