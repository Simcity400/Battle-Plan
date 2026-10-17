# dashboard-equipment-losses

The equipment losses product: one country's equipment losses over a branch, as a
tree of cause, equipment type and exact design. Behaviour is documented beside
the code in [products/equipment_losses.rs](src/products/equipment_losses.rs) and
in the widget guides ([equipment
losses](../../ui/src/widgets/equipment-losses/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-equipment-losses` builds and runs only this crate's
tests. The `test-support` feature compiles the frame-at-a-time observer the
dashboard's pipeline tests drive.

The prepared observer selects Combat Log rows by their raw owner's recorded-key
range, so changes and pruning of other owners' rows rebuild no log index.
Amount-only destruction deltas with unchanged keys reuse the owner join and
availability operands; every hour still consumes its signed drain, including an
equal one. Country balance changes keep the slot projection, and unchanged typed
read keys keep their saved serialized text. These joins and comparisons rebuild
at resume; recorded rows, published pages and saved loss operands keep their order.
