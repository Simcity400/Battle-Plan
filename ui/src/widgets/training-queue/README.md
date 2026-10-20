# Training queue

## What it shows

A country's deployment orders in the game's queue order, each a template ordered in some amount, with the divisions in training nested under it: training, equipment fill against the template's need, and manpower available against needed. Each order also shows its location and priority as the deployment view names them.

## Products it reads

[`training-queue`](../../../../crates/dashboard-army/src/products/training_queue.rs): the daily queue read at or before the shown hour, dated with that read.

## Settings

One country; Campaign, Follow (default) or Pin; density, cell bars and When narrow. There is no sort: the game's order is the meaning.

## Notable behaviour

- Orders use the shared table's group header rows; they start open and close per session. A closed order shows how many divisions are in training and their lowest equipment fill.
- Values are daily samples; tooltips say "As of" the read. No ending date, readiness or deployment is derived; an amount ordered never becomes invented divisions.
- A stored halting reason is shown only as *Halting reason not available*: no verified mapping names it.
- Equipment fill counts no type past its need, so a surplus of one type never covers another's shortfall; an unknown need, or a held design whose type is unresolved, reads "—" with the known needs in the tooltip.
- The country is the one identity the choice named at the date it was made, kept with the widget and followed through that identity's renames only, so a later holder of the tag never takes it; a tag that named no country then and several over the campaign asks which one.
- Location is the display name of the state owning the order's province (the province id on hover); priority is the localised tooltip of the deployment view's priority button. Narrow widths hide Priority, Location, Manpower and Equipment, in that order.

## Files

| File | Responsibility |
|---|---|
| [TrainingQueueWidget.tsx](TrainingQueueWidget.tsx) | Nested table, header pill and states. |
| [trainingQueueData.ts](trainingQueueData.ts) | Validation, rows, fills and tooltips; adjacent tests. |
