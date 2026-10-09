# Equipment produced

## What it shows

Country production as a race, lines or one-country mix, with type/variant/arm breakdowns.

## Products it reads

[`equipment-produced`](../../../../crates/dashboard/src/products/equipment_produced.rs) counts actual units delivered by production ticks, including deleted lines’ final publications. Shared [equipment helpers](../../dashboard/equipment.ts) supply content naming rules.

## Settings

Top 10 by default, manual countries or groups; running total/week/month, grouping, included types, refits (off), split bars, playback/axes and drill choices.

## Notable behaviour

- Counts begin at the recorded observation start. Week/month need both endpoints in one continuous holder/observation; short or broken windows are unknown. Lost ticks or final publications leave affected totals unknown.
- Rocket deliveries retain country/design totals after site removal in the same groups. Combined totals need both sources' coverage; while rocket coverage is unknown (for example the partial interval after a world reset or load), combined values stay unknown and equipment-line subtotals remain readable.
- Top N excludes zero; manually chosen known zero stays zero. Refits count only when enabled. Lines unseen by the census count as new production.
- Unresolved nonzero/unknown types can make resolved group attribution unknown while the independent country total remains exact. An included-type filter can make that total unknown too. Variant names never use future reads.
- Variant colours/folding belong to each country. Keep compact delta histories, without samples × designs.
- Drills preserve signed rows, zero, exact residuals and unknowns; geometry alone uses positive amounts. A picked day holds; Follow resumes playback. Pin binds the clicked native holder/date through aliases, excluding replacement holders.

## Files

| File | Responsibility |
|---|---|
| [EquipmentProducedWidget.tsx](EquipmentProducedWidget.tsx) | Shared race view and drill composition. |
| [producedData.ts](producedData.ts), [producedDrill.ts](producedDrill.ts) | Compressed exact values, groups, actor-bound pins and history; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [equipmentProduced.css](equipmentProduced.css) | Options, content choices, registration and layout. |
