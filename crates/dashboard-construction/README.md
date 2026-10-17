# dashboard-construction

The construction log and construction spending products, in one crate because
each names the other: every building and railway project of every country's
construction queue, and the civilian factory output spent on each. Behaviour is
documented beside the code in
[products/construction_log.rs](src/products/construction_log.rs),
[products/construction_spending.rs](src/products/construction_spending.rs) and
in the widget guides ([construction
log](../../ui/src/widgets/construction-log/README.md), [construction
spending](../../ui/src/widgets/construction-spending/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the modules
keep their dashboard paths and name what they use at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports them at their old
`dashboard::products::` paths, registers them, and prepares and queries them in
its pipeline.

`cargo test -p dashboard-construction` builds and runs only this crate's tests.

The newest construction window can be queried independently of the immediate
prefix manifest. Its look-back starts with the last two blocks and extends to
recorded project creation and output-chain supplying hours until every project
whose row or output appears in the window has an anchored first queue read and
output history. Reading from the branch's start is the forward observation.
An unreadable look-back publishes no window. Numeric results are retained per
source revision, so re-asks only remap type metadata. A window carries its own
`types` and `exactFrom`; it uses only building identities whose first spelling
the forward catalogue has already resolved, otherwise it remains pending.
Long projects and old output chains extend the look-back without a chosen cap.
Prefix project histories and spending work intervals remain visibly approximate
until complete, with later hours pending.

Before the first catalogue, manifests still list the recording's windows with
no accepted projects or types. Spending has no output reads or leaves. An
unproved, uncovered log window has empty projects and observations with the
shared prefix's pending hours, rather than asserting that construction was not
recorded. Published prefix presentations retain their approximate marking.
