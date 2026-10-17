# Equipment losses

## What it shows

One country’s signed losses by cause → equipment type → exact design, with a compact completeness status and the full loss inventory.

## Products it reads

Prepared [`equipment-losses`](../../../../crates/dashboard-equipment-losses/src/products/equipment_losses.rs) requests use one viewport plus exact reached dates. The existing product owns native cause meanings, content labels and coverage. Background preparation observes all native holders once; queries read the selected holder's sparse loss series and metadata.

## Settings

Land losses from Every destruction (default) or Game’s Combat Log; hierarchy levels, shared sunburst controls, drill placement and Show. Campaign/follow/pin/range are available.

## Notable behaviour

- Air uses destruction in both modes; recovered equipment and lend-lease sent are excluded. Names use supplying reads and the content stack; unavailable amounts retain their missing-value states without acquisition diagnostics.
- Destruction starts at the first complete branch/load drain, excluding the opening partial interval. Later lost drains/writers leave an incomplete subtotal, never a bound; complete reads cannot recover it. Completeness stays visible with legends off; affected amounts show short unavailable or incomplete statuses.
- Combat Log may retain earlier losses. Runtime feature/country gates and hourly orders-group census determine coverage; captured amounts still count. Inclusive ranges subtract before their start; Follow keeps the counting origin. Outside-branch pins/ranges refuse.
- BigInt preserves every signed digit. Negative corrections and recorded zero remain selectable; only positive geometry draws. Corrections or incomplete full totals disable shares/Fill.
- Leaves drill native cohorts; parents zoom. History steps at supplying changes and breaks at unread intervals. Picking holds; Follow uses the clock. Range drills keep their subtraction basis without Follow.
- Pin saves campaign/branch, native slot/ID/origin, cohort and date/range basis. Validated aliases follow only that holder; gaps/reused tags cannot supply losses. Headers follow the body date. Invalid holders refuse; older missing-holder pins keep neutral headers.
- NumericReply retains picks through content-only refresh; source or native-selection changes invalidate them.
- Chart geometry uses viewport summaries and the existing stack painter. Hover and picks require separately requested exact dates. Keyboard navigation reads shared series change boundaries and native sinking dates. Pending playback stays at the previous reply's own hour.
- Sparse ship events publish at observation time and retain the native sinking date, preserving backdated totals without rewriting committed preparation.
- Totals require prior drains, retained closed log entries, convoy reset increments, ended exchanges and ship observations, so one-frame samples cannot supply them. Before preparation reaches the shown hour, the reply answers its prepared prefix: the totals lost through the prefix's last hour, with the note "Lost through <date>; the rest is still being prepared."; the history ends there and a range starting after it reads "This range is still being prepared." Until the first prefix the shared "still being prepared" note shows.

Before preparation completes, the sunburst and history explicitly say Approximate:
a later ship observation can add a sinking at an earlier hour, and a later token
can change an equipment type. Every recorded prefix hour is provisional because
neither source has a finite revision bound. The history fades those spans with a
dashed key; completion restores exact presentation. Land coverage on the wire is
`landCoverage`, independently of preparation progress.

## Files

| File | Responsibility |
|---|---|
| [EquipmentLossesWidget.tsx](EquipmentLossesWidget.tsx) | Sunburst, coverage and inventory/drill composition. |
| [lossesData.ts](lossesData.ts), [lossDrill.ts](lossDrill.ts) | Exact amounts, coverage, scoped selections and history; adjacent tests. |
| [lossesParity.ts](lossesParity.ts) | Test-only complete Sunburst inputs, exact source points, totals, content and coverage. |
| [preparedLosses.ts](preparedLosses.ts) | Prepared viewport routing, reached dates, cancellation and navigation. |
| [definition.ts](definition.ts), [index.ts](index.ts), [equipmentLosses.css](equipmentLosses.css) | Settings, registration and layout. |

Parity builds expose every ordered Sunburst item and its exact signed operands,
native country selection, counting window, coverage and referenced content.
The provider starts no reads. Ordinary builds expose no model provider.
Its label is "Equipment losses"; prepared replies identify their exact reached
dates separately from chart samples.
