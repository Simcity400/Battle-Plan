# Production spending

## What it shows

Signed factory IC applied to equipment lines and rocket sites, grouped as production source → equipment type → exact design.

## Products it reads

[`production-spending`](../../../../crates/dashboard/src/products/production_spending.rs) shares Equipment produced’s branch scan and content helpers. [line_output_sites.rs](../../../../crates/engine/src/line_output_sites.rs) defines line-output hook sites.

## Settings

Campaign/follow/pin/range; refitted ships (off), hierarchy levels and shared sunburst options. One country and whole campaign are the defaults.

## Notable behaviour

- Counts applied input even without delivery; later design/cost changes never reprice it. Instant progress is excluded. Rocket country/design totals survive site removal with independent coverage; while rocket coverage is unknown (for example after a world reset or load), equipment subtotals stay readable.
- “No military production yet.” requires a dated complete-empty census/output read and independently known rocket absence. Empty leaves, stalled lines, zero signed totals and empty ranges do not prove this state. Historical spending survives deleted lines; unread sources retain their reasons.
- Final deleted-line input remains counted. Ranges subtract signed endpoints in one uninterrupted observation. Lost publications, resets, overflow or classification failure leave spending unknown independently of delivery counts.
- Unresolved types keep tokens/values and can make group attribution unknown while an independently measured country total stays exact. Names are read-dated; the rocket source name uses recorded building classification. Design/source pairs remain distinct when rings are hidden.
- Signed corrections keep every fraction in totals/legend rows; only positive amounts draw. Unknown totals supply no shares or Fill denominator. The shared Sunburst owns zoom/folding; this widget adds no dated Jump menu.

## Files

| File | Responsibility |
|---|---|
| [ProductionSpendingWidget.tsx](ProductionSpendingWidget.tsx) | Product and Sunburst adapter. |
| [spendingData.ts](spendingData.ts) | Exact amounts, hierarchy, attribution unknowns and footnotes; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Levels, shared options and registration. |
