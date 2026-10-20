# Army losses by month

## What it shows

A country's army losses per calendar month from the game's retained Combat Log, one stacked bar per month. Manpower stacks by the log's source channels (Ground, Air, Navy) or by army group; Equipment shows one ledger at a time (Lost in combat, Lost to attrition, Recovered, Enemy losses dealt), stacked by equipment type or army group.

## Products it reads

[`army-losses`](../../../../crates/dashboard-land-battles/src/products/army_losses.rs): the retained log at the shown hour, whose months are each the latest version the game keeps.

## Settings

One country; Measure (also the caption switch); Stack; Ledger (also a caption switch, Equipment only); drill placement; Campaign (default), Follow, Pin or Range.

## Notable behaviour

- The country total adds every distinct army's log once; army groups keep no log of their own, so no parent is added beside its armies ([countedArmies](armyLossesData.ts)).
- *By army group (current)* assigns each army to its army group now, with *Independent armies* and *Army group not recorded* buckets; the organisation when a loss was charged is not recoverable.
- Lost to attrition adds the three attrition ledgers, which partition attrition. Recovery and losses dealt are never netted against losses.
- A month with no retained entry is hatched as unknown: the game pruned it or never wrote it. The chart keeps the whole time range, so pruned months show as hatched rather than shrinking it.
- An equipment month any counted army's ledger holds unwitnessed amounts for is hatched too: the other armies alone are not its total. Older months keep the amounts last witnessed while they could change.
- Manpower adds as the game's signed 32-bit counters do, wrapping rather than widening. Equipment adds its exact stored amounts through every stack, the Other fold and tooltip totals; only the bars' geometry uses numbers.
- The country is the one identity the choice named at the date it was made, kept with the widget and followed through that identity's renames only, so a later holder of the tag never takes it; a tag that named no country then and several over the campaign asks which one.
- The month open at the playback date is marked partial in its tooltip. Clicking a month opens its armies with their channels and ledgers.
- Signed values are kept; a negative correction draws below zero.

## Files

| File | Responsibility |
|---|---|
| [ArmyLossesWidget.tsx](ArmyLossesWidget.tsx) | Chart, caption switches, legend and month drill. |
| [armyLossesData.ts](armyLossesData.ts) | Validation, the counting rule, monthly stacks and tooltips; adjacent tests. |
