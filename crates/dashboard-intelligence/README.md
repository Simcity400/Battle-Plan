# dashboard-intelligence

The intelligence products: agency comparison, the espionage log, decryption,
the operatives roster and operative placements, with the intelligence and
operative content they read, the operation descriptions, and the intelligence
observer that prepares all five from one pass over each frame. Their behaviour
is documented beside the code in [products/](src/products/) and in the widget
guides ([agency comparison](../../ui/src/widgets/agency-comparison/README.md),
[espionage log](../../ui/src/widgets/espionage-log/README.md),
[decryption](../../ui/src/widgets/decryption/README.md),
[operatives roster](../../ui/src/widgets/operatives-roster/README.md),
[who spies on whom](../../ui/src/widgets/who-spies-on-whom/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md). The
modules keep their dashboard paths, `products::<name>`, and name what they use
at the crates that own it (dashboard-base, dashboard-text and dashboard-core). The [dashboard](../dashboard/README.md)
re-exports them at `dashboard::products::<name>`, registers them, checks their
content, and prepares and queries them in its pipeline. The crate implements
`ReplyWarnings` for its replies.

`cargo test -p dashboard-intelligence` builds and runs only these products'
tests. The `test-support` feature compiles their test content for the
dashboard's content-check and pipeline tests.
