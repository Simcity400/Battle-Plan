# dashboard-unit-leaders

The unit leaders product: every military leader's career, skills and traits from
hourly reads, and their portraits. Behaviour is documented beside the code in
[products/unit_leaders.rs](src/products/unit_leaders.rs) and in the widget
guides ([generals](../../ui/src/widgets/generals/README.md), [general
career](../../ui/src/widgets/general-career/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

The family is prepared on its own: [products/unit_leaders/block.rs](src/products/unit_leaders/block.rs)
holds its pre-pass extract (a block read by the fields leaders read, with the
tables its run binds from the content, kept as whole, reduced and quiet hours
with the block reader's change facts) and its
fold on the carried prepared state, described in the
[dashboard's prepared leaders](../dashboard/README.md#prepared-leaders).

`cargo test -p dashboard-unit-leaders` builds and runs only this crate's tests.
The `test-support` feature compiles the outcome frames
(`prepared::test_fixtures`) the dashboard's pipeline tests build on and the
whole-recording loader, the parity oracle they compare against.
