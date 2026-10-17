# dashboard-resource-balance

The resource balance and trade flows products, in one crate because trade flows
builds on the balance: where each of one country's strategic resources comes
from and goes, and every resource trade deal as a flow from seller to buyer.
Behaviour is documented beside the code in
[products/resource_balance.rs](src/products/resource_balance.rs),
[products/trade_flows.rs](src/products/trade_flows.rs) and in the widget guides
([resource balance](../../ui/src/widgets/resource-balance/README.md), [trade
flows](../../ui/src/widgets/trade-flows/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the modules
keep their dashboard paths and name what they use at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports them at their old
`dashboard::products::` paths, registers them, and prepares and queries them in
its pipeline.

Changed trade exchanges update only their flows' native endpoints. Other active
flows hold through the accepted frontier and materialize before a whole-table
observation or publication. Shared coverage and refusal clocks advance separately.
Hourly signed trade integrals still consume every active flow's terms.

`cargo test -p dashboard-resource-balance` builds and runs only this crate's
tests.

Trade retains its current published catalogue text and compressed ranges
between flushes, applying the shared patch and compaction rules without
rereading its own publication. Restoration validates the chain again. Numeric
trade channels pass one explicit payload table through a flush, with
the same separate per-key payloads and saved continuation.

Catalogue reuse checks every live range's immutable identity and the next
empty/missing slot in the current writer. A second publisher, store
replacement or abandoned writes invalidate the cache; an unchanged append
can retain it. The shared publication rules are in
[dashboard-base](../dashboard-base/README.md#prepared-series).
