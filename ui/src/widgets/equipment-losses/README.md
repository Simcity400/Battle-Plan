# Equipment losses

## What it shows

One country’s signed losses by cause → equipment type → exact design, with a compact completeness status and the full loss inventory.

## Products it reads

[`equipment-losses`](../../../../crates/dashboard/src/products/equipment_losses.rs) owns native cause meanings, content labels and coverage.

## Settings

Land losses from Every destruction (default) or Game’s Combat Log; hierarchy levels, shared sunburst controls, drill placement and Show. Campaign/follow/pin/range are available.

## Notable behaviour

- Air uses destruction in both modes; recovered equipment and lend-lease sent are excluded. Names use supplying reads and the content stack; unavailable amounts retain their missing-value states without acquisition diagnostics.
- Destruction starts at the first complete branch/load drain, excluding the opening partial interval. Later lost drains/writers leave an incomplete subtotal, never a bound; complete reads cannot recover it. Completeness stays visible with legends off; affected amounts show short unavailable or incomplete statuses.
- Combat Log may retain earlier losses. Runtime feature/country gates and hourly orders-group census determine coverage; captured amounts still count. Inclusive ranges subtract before their start; Follow keeps the counting origin. Outside-branch pins/ranges refuse.
- BigInt preserves every signed digit. Negative corrections and recorded zero remain selectable; only positive geometry draws. Corrections or incomplete full totals disable shares/Fill.
- Leaves drill native cohorts; parents zoom. History steps at supplying changes and breaks at unread intervals. Picking holds; Follow uses the clock. Range drills keep their subtraction basis without Follow.
- Pin saves campaign/branch, native slot/ID/origin, cohort and date/range basis. Validated aliases follow only that holder; gaps/reused tags cannot supply losses. Headers follow the body date. Invalid holders refuse; older missing-holder pins keep neutral headers.
- NumericReply retains picks through content-only refresh; source or numeric changes invalidate them.

## Files

| File | Responsibility |
|---|---|
| [EquipmentLossesWidget.tsx](EquipmentLossesWidget.tsx) | Sunburst, coverage and inventory/drill composition. |
| [lossesData.ts](lossesData.ts), [lossDrill.ts](lossDrill.ts) | Exact amounts, coverage, scoped selections and history; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [equipmentLosses.css](equipmentLosses.css) | Settings, registration and layout. |
