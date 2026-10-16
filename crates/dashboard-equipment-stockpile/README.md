# dashboard-equipment-stockpile

The equipment stockpile product: what one country holds of each variant over a
branch, what its production lines make of it, and the dated best-variant
artwork. Behaviour is documented beside the code in
[products/equipment_stockpile.rs](src/products/equipment_stockpile.rs) and in
the widget guides ([equipment
stockpile](../../ui/src/widgets/equipment-stockpile/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-equipment-stockpile` builds and runs only this crate's
tests.
