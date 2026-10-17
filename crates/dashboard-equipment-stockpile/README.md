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

Prepared holders catch up through an authenticated span in one step, retaining
the same open-run endpoints and saved state as advancing every hour. Identity
changes and breaks are observed at their own hours between those spans. Numeric
reads use recorded row deltas when present; a changed table without them,
or a change to table availability, holders or dictionary, rebuilds the whole
projection. Continuation JSON uses the hourly branch writer's shared buffered
compression helper.

`cargo test -p dashboard-equipment-stockpile` builds and runs only this crate's
tests.

A table sampled before complete preparation retains exact supplying-read
quantities, with its type attribution marked approximate at the requested hour.
The complete branch can supply a later nonempty classification token. The table
shows this distinction without changing the supplying-read sampler.

The at-hour sampler also answers before the first commit, with the shared empty
accepted extent. Its raw reply is retained in the existing observation cache,
bound to content and keyed by holder tag, hour and membership spans. Re-asks
update preparation metadata without repeating the supplying-read look-back.
The retained-byte bound follows the live blocks in that read window.
