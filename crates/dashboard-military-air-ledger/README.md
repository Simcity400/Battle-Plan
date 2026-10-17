# dashboard-military-air-ledger

The military air ledger product: exact observed aircraft losses and the building
damage dealt by aircraft bombing. Behaviour is documented beside the code in
[products/military_air_ledger.rs](src/products/military_air_ledger.rs) and in
the widget guides ([air kills
losses](../../ui/src/widgets/air-kills-losses/README.md), [bombing
damage](../../ui/src/widgets/bombing-damage/README.md), [bombing damage
breakdown](../../ui/src/widgets/bombing-damage-breakdown/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-military-air-ledger` builds and runs only this crate's
tests. The `test-support` feature compiles the whole-recording loader, the
parity oracle the dashboard's pipeline tests compare against.

Prepared ledger replies expose the committed prefix with root `coverage` and
`pending` hours. Before the first commit, an empty marked reply has null coverage;
it asserts no observed interval or total. Signed events, native dates, supplying
reads, ranking and refusal rules use the same saved observer state and reply
builder as a complete reply. A shown hour beyond that prefix is still being
prepared in each of the three widgets.
