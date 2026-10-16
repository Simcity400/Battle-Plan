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

`cargo test -p dashboard-unit-leaders` builds and runs only this crate's tests.
The `test-support` feature compiles the outcome frames
(`prepared::test_fixtures`) the dashboard's pipeline tests build on and the
whole-recording loader, the parity oracle they compare against.
