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

Requested-hour war replies include each native side's participant contribution breakdown, total and participation share. They read directional ledgers and native delivery histories independently of the structural history. Capture retains nonzero histories for war-roster owners; the common screen/hover projection selects only actual displayed same-side occurrences, excludes self/opponents and preserves retained/asymmetric roster behaviour. Factors come from the verified content stack; unavailable tables or missing ledgers refuse the projection. Missing rows in a complete sparse delivery-history table certify exact zero. No contribution series is duplicated into prepared war continuity.

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
Unknown-loss agreement scratch is local to a read; continuations retain its
settled total and seen flag, and each new read initializes its own scratch.

`cargo test -p dashboard-wars` builds and runs only these products' tests. The
`test-support` feature compiles their test oracles, instrumentation and the
casualty fixtures (`war_casualties::test_fixtures`) for the dashboard's tests.

Wars, casualty states, pair manifests/windows and breakdowns answer before the
first commit with empty ordinary replies and the shared empty prefix extent.
No cumulative loss is supplied for those pending hours. The wars' independent
at-hour groups remain available without continuity lanes; published prefixes
use the existing observers and projections.

The interactive `wargoals` product supplies the Diplomacy inventory and the expandable Wars panel at a requested country/hour. It preserves full target lists and unused membership, resolves type names through recorded content and derives native completion display from stored operands. Eligibility is derived only for displayed inventory pairs: a recorded live war or shared nonzero faction blocks justification and declaration. Remaining native gates stay unresolved, separately named and counted for each pair; inventory availability never implies action eligibility.

Definition terms come from the recorded content stack for both owned goals and active justifications: annexation, peace-term limits and factors, default expiry and generation operands. A justification's authored default expiry is shown without predicting its eventual grant time. Conditional term results and historical upfront political-power amounts have named, counted refusals: an inventory entry contains neither a peace action's evaluation context nor a historical payment amount. Actor, start, progress, completion and tension remain visible independently.

War-goal type names localize the definition's declared spelling after token lookup, independently of the recorded token's letter case, with ACTOR and TARGET resolved through recorded country naming operands. Authored allowed/available blocks use the shared trigger-description formatter: allowed describes the actor; available describes the target with the actor as ROOT. Unsupported descriptions are named and counted, and missing descriptions stay distinct from empty authored conditions. The product selects the shared country-name and trigger input tables at its requested hour; it adds no capture work.
Casualty block folds keep the carried first read's unknown-loss agreement for countries with no later changed relation in that block. Later touched countries take the extract's last agreement, with totals translated onto the carried history.

War page queries read only the continuation's counter operands, leaving its
input snapshot to preparation and the native walk. Structure-only projections,
including casualty breakdown's war grouping, read no counter operands. They
retain the same whole-branch native viewpoint walk and grouping history.
War page queries read only the continuation's counter operands, leaving its
input snapshot to preparation and the native walk. Structure-only projections,
including casualty breakdown's war grouping, read no counter operands. They
retain the same whole-branch native viewpoint walk and grouping history.
