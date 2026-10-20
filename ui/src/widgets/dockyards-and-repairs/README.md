# Dockyards & repairs

**What it shows:** One country's ports at the hour shown, each a group with its repair dockyards and priority, and the ships queued there for repair in the game's order with their hull; a totals line counts the ships queued and the repair dockyards in use.

**Product(s) it reads:** [`naval-repairs`](../../../../crates/dashboard-naval-yards/README.md), a keyed family prepared from the recorded repair allocation, bases, queues and ships. One request per country holds the branch; playback looks the hour up locally.

**Settings:** One country (the dashboard's by default); the shared table settings (cell bars, density, When narrow). Follow (default), Pin or the campaign's end.

**Notable behaviour:**

- Queue order and repeated references are the recorded ones; ports are grouped rows that open and close (the shared table's group rows) and the table keeps the game's order.
- Hull is the ship's recorded hull against its maximum. No repair rate or days left: the recording has neither. Port and fleet repair policies are in tooltips only.
- Ships are shown by their recorded name where the recording holds one, else by their design's name, with the full ship reference on hover.
- Dockyards and priority are the low-priority columns: they hide first as the widget narrows, their values moving into the row tooltip.

**Files:** [DockyardsWidget.tsx](DockyardsWidget.tsx) draws the grouped table; the shared lookups are in [navalYards.ts](../../dashboard/navalYards.ts) and [refits/shipNames.ts](../refits/shipNames.ts), the request in [refits/useYards.ts](../refits/useYards.ts).

Shared framework conventions: [widgets](../README.md).
