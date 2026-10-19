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
Signed trade integrals retain each flow's captured integer rates until a rate
changes or a publication materializes them. Between changes the exact prefix
is the preceding total plus rate times accepted hours. No division or rounding
occurs during accumulation: rounding is already in the recorded integer rate.
Gaps close the accepted span before its rates are replaced. Product overflow
with representable hourly prefixes takes ordinary checked addition.
Publication retains every existing hourly staircase row, with grouped per-key
pushes; zero rates retain one constant run. Runtime rate spans rebuild after
restore, and saved series and continuation layouts are unchanged. A separate
compact counter charges exactly the rows hourly integration would have pending,
including skipped zero rates and held endpoints sealed by gaps. Deferred rows
and rate-cache memory do not contribute to the publication budget.

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

Resource query tails retain numeric block folds and learned resource names
together in the bounded observation cache. Within an ask, equal supplying table
views reuse their numeric projection; the clock is not a numeric operand.
Selected-country pool reads build operands only for the selected native holders.
Trade query samplers retain their reduced exchanges, resource dictionary and
country-holder inputs for the reply, refreshing them from every visited frame's
actual table changes. Reached-hour refusals and the ordinary holder and flow
projections still run at each visited hour.

The `market-lendlease` product reconstructs only the selected hour and demanded tables, sharing one source-local frame for the Trade flows Market & lend-lease view and its catalogue variant card. It preserves unavailable collections and exact integer strings and resolves demanded equipment/subsidy names through recorded content.

The market-lendlease product reads the complete MarketEvents ledger, including
explicit holder-level amount-refusal rows. It retains named amount refusals for the affected contract
or agreement alongside complete fuel, payment and lifetime occurrences; missing
amounts never become a zero or an empty observed-delivery list.
