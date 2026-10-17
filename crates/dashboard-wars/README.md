# dashboard-wars

The wars and war casualties products, in one crate because each names the other:
each war's sides, participants, names and outcome, and the casualties each
country took and dealt, in total, between pairs and by domain. Their behaviour is
documented beside the code in [products/wars.rs](src/products/wars.rs) and
[products/war_casualties.rs](src/products/war_casualties.rs), and in the widget
guides ([wars](../../ui/src/widgets/wars/README.md),
[casualties](../../ui/src/widgets/casualties/README.md),
[casualties between](../../ui/src/widgets/casualties-between/README.md),
[casualties breakdown](../../ui/src/widgets/casualties-breakdown/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
modules keep their dashboard paths, `products::wars` and
`products::war_casualties`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)). The [dashboard](../dashboard/README.md) re-exports both modules at
their old `dashboard::products::` paths, registers them, and prepares and
queries them in its pipeline.

Casualty-only war observations reuse the normalized relation topology and
its counter keys in native relation order. A new join rebuilds those keys;
block folds invalidate the extract's local numbering. The casualty scan
likewise retains its supplying-pair set until its joined lifetime keys
change, including across a non-read hour. Hourly counter, unknown-loss and
coverage steps still run with their original supplying evidence.

`cargo test -p dashboard-wars` builds and runs only these products' tests. The
`test-support` feature compiles their test oracles, instrumentation and the
casualty fixtures (`war_casualties::test_fixtures`) for the dashboard's tests.

Wars, casualty states, pair manifests/windows and breakdowns answer before the
first commit with empty ordinary replies and the shared empty prefix extent.
No cumulative loss is supplied for those pending hours. The wars' independent
at-hour groups remain available without continuity lanes; published prefixes
use the existing observers and projections.
