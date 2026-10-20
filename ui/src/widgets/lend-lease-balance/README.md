# Lend-lease balance

## What it shows

Each country's cumulative lend-lease received (left) and sent (right) up to the date shown, as Casualties' butterfly, ranked by both (top ten by default). The Measure switch (caption and settings) picks the game's own recorded measures, Industry or Fuel; no app-computed equivalent is offered. A country's bar opens its partners in the drill: its record of each partner at the drill date, and its sent and received totals over time.

## Products it reads

[`lend-lease-balance`](../../../../crates/dashboard-economy/src/products/lend_lease_balance.rs), prepared on its own (family `lend_lease_balance`) from `WarLendLeaseHistories`: the cumulative histories the war overview keeps for countries named by recorded war rosters. A country's totals sum its own records only, so a delivery is never counted from both sides.

## Notable behaviour

- A group whose membership was not recorded at the hour shown says so, and a group with no members then says it has none; neither reads as "none" for its countries.
- A country the table does not list (outside the war rosters) has no value and leaves the chart: its history is not recorded, not zero.
- An empty chart says "No lend-lease yet" only when every selected country (a group's members at the hour, as the bars resolve them) has recorded totals of exactly zero then; with some recorded and some not, it says so, and a group whose membership was not observed says that.
- Histories can predate the recording; the tooltip says "Not recorded before <date>" for the first read. Signed corrections stay exact in the values; only positive bars draw.
- Ties rank by native holder, so order is deterministic. The menu jumps to the first recorded read.
