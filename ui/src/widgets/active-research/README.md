# Active research

## What it shows

One compact card per research slot open at the hour shown: the technology it researches with its research folder, its latest read progress, the game's days-left estimate and an Ahead of time badge, or Idle with the days saved for the next research. The order and dates of past research are the [Research history](../research/README.md) widget.

## Products it reads

The prepared research query, using the [`research`](../../../../crates/dashboard-base/src/products/research.rs) observer and reply presentation shared with Research history, and [`technology-artwork`](../../../../crates/dashboard/src/products/technology_artwork.rs) for the pictures.

The cards ask `series-state` for family `research`, with `reads: [{tag, hour}]`.
Beyond the prepared prefix, the production observer reads only the supplying
period and its previous daily progress, checking every intervening hour's owner
read and identity. Slot state, latest-read progress and the Ahead-of-time badge are exact. Days
left is exact at a supplying read; during a carry the older countdown segment
phase remains still being prepared. A gap or missed supplying read stays unknown.
Assignment dates and the time breakdown remain visibly "Still being prepared";
Jump is disabled until assignment history is available. Gate coverage changes
refresh the cards, and prepared history supplies their attribution.

## Settings

Country and campaign/follow/pin; Live is the default. No widget-specific options.

## Notable behaviour

- Progress is the latest read of that assignment, dated in the tooltip, and never interpolated between reads. Days left is the game's estimate at the hour's research speed.
- The badge shows while the ahead-of-time penalty applies at that hour; the tooltip names the technology's year.
- A card opens the technology's time breakdown; right-click or Shift+F10 offers Jump to its earliest joined assignment, including earlier slots.
- At an hour without a read, the slots open at the last read are Unavailable; a slot that had closed is not shown.
- Older recordings that do not identify the assignment still show its name, folder, read progress and the game's estimate; the card has no breakdown.

## Files

| File | Responsibility |
|---|---|
| [ActiveResearchWidget.tsx](ActiveResearchWidget.tsx) | Cards, tooltip and breakdown popover. |
| [useActiveResearch.ts](useActiveResearch.ts) | Reached-state requests and validation; adjacent tests. |
| [activeResearchData.ts](activeResearchData.ts) | Each slot's state at the hour; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [activeResearch.css](activeResearch.css) | Metadata, registration and layout. |

Parity builds expose the "Active research" model, including exact slot state, progress, countdown and saved-day values.
