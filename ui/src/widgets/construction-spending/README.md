# Construction spending

## What it shows

One country’s signed civilian-factory output as a sunburst: new/repair work → building type → state.

## Products it reads

[`construction-spending`](../../../../crates/dashboard/src/products/construction_spending.rs) shares the Construction log scan. [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies state names at the shown hour.

## Settings

Campaign/follow/pin/range; each hierarchy level and the shared [sunburst options](../../charts/sunburst/sunburstOptions.ts).

## Notable behaviour

- Measures recorded factory IC, including final deleted-line publications. Never derive it from finished levels or sampled cost. Bank withdrawals, instant progress and railway carry are excluded.
- Exact signed fractions remain in totals and selectable correction rows; only positive amounts draw slices. Natural repairs can count as projects with zero factory IC.
- Amounts enter at supplying reads. Missing final publications, resets or unidentifiable between-read output leave the country whole unknown while retaining its known prefix. Older recordings can open without output.
- A project contributing to both new and repair counts once when that level is hidden. [useStateNamesAt](../../dashboard/useStateNamesAt.ts) changes dated labels, never paths/amounts. Older uncaptured name fields use authored fallback; captured failures keep ID/reason.

## Files

| File | Responsibility |
|---|---|
| [ConstructionSpendingWidget.tsx](ConstructionSpendingWidget.tsx) | Product, dated naming and shared Sunburst wiring. |
| [spendingData.ts](spendingData.ts) | Exact totals, project counts, unknowns and hierarchy; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings and registration. |
