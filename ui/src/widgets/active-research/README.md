# Active research

## What it shows

One compact card per research slot open at the hour shown: the technology it researches with its research folder, its latest read progress, the game's days-left estimate and an Ahead of time badge, or Idle with the days saved for the next research. The order and dates of past research are the [Research history](../research/README.md) widget.

## Products it reads

The prepared research query, using the [`research`](../../../../crates/dashboard-base/src/products/research.rs) observer and reply presentation shared with Research history, and [`technology-artwork`](../../../../crates/dashboard/src/products/technology_artwork.rs) for the pictures.

Until the requested history is prepared, the shared preparation status says
"This range is still being prepared." Slot segments, assignment attribution and
carried progress require the research observer's earlier reads.

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
| [activeResearchData.ts](activeResearchData.ts) | Each slot's state at the hour; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [activeResearch.css](activeResearch.css) | Metadata, registration and layout. |

Parity builds expose the "Active research" model, including exact slot state, progress, countdown and saved-day values.
