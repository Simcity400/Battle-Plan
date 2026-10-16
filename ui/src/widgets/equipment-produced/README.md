# Equipment produced

## What it shows

Each country's equipment production as a race or lines over time.

- **Units:** one equipment type at a time, because units of different equipment never add up.
- **IC:** the factory IC put into equipment. IC is one currency, so every type adds up.

A caption line says what is counted, over which window and to which date. A country's breakdown lists every type it made, with its models, units and IC.

## Products it reads

- [`equipment-produced`](../../../../crates/dashboard-base/src/products/equipment_produced.rs) counts each model's units delivered by production ticks, including deleted lines' final publications. With `count: ic` it counts the exact factory IC applied to the lines instead: the input Production spending shows, excluding instant progress. It also ranks the top N.
- Prepared race views read exact endpoint pages through `production-select`; only the visible country range is requested. Type choices use the prepared branch catalogue. Lines, opened and pinned breakdowns use the whole-history product. The default units type retains the all-types total query so it stays the type delivered most; running-total captions keep the catalogue's global observation. An unknown empty endpoint uses the history view to preserve absent history and unread-current-value states.
- [Equipment helpers](../../dashboard/equipment.ts) supply content naming rules.
- The breakdown's pictures come from `stockpile-artwork`, through [useStockpileArtwork](../equipment-stockpile/useStockpileArtwork.ts): the drilled country's best design for each type.

## Controls and settings

**Caption line:**
- The equipment pill: a searchable single-choice list of every type delivered in the recording. In IC it adds All equipment.
- The caption, for example "Units delivered on 22 Jun 1944", "IC put into equipment in the month to …" or "… from 24 Oct 1943 to …".
- The Units / IC control.

**Settings:**
- View: Race or Lines over time.
- Count: Units or IC.
- Equipment: the pill's choice. Several types add up on purpose.
- Measure: Per day (default), Per month or Since recording start.
- Refits (off), the shared race time, axis and breakdown settings, and Top 10 by default, manual countries or groups.

## Notable behaviour

**What is shown by default:**
- In units, until a type is chosen, the widget shows the type the countries shown delivered most, read from an all-types reply. In IC it shows all equipment.
- The date is playback while Live, else the campaign end. The breakdown header uses the same date.

**Measures:**
- Per day is one production tick: the running total minus the read 24 hours before.
- Per month needs both ends of the window in one continuous holder and observation. A short or broken window is unknown, never zero.
- Counts begin at the recorded observation start. Lost ticks or final publications leave the affected totals unknown.

**Exact values and formatting:**
- Values stay the product's exact integers; charts plot them in the count's unit.
- Units are whole. IC and every per-day rate follow `formatRate`: whole from 10, two significant figures below.
- Units requests leave `count` out, so a bridge from before IC answers them too.

**Unknown and excluded values:**
- Top N excludes zero; a manually chosen known zero stays zero. Refits count only when enabled. Lines unseen by the census count as new production.
- Rocket deliveries keep country and design totals after site removal. Combined values need both sources' coverage. Equipment-line subtotals stay readable in tooltips without being added in.
- A type is unknown while a model of an unresolved type holds anything, since it could belong to it. The independent country total stays exact unless an included-type filter could hold it.

**The breakdown:**
- Its table lists every type delivered or still being built, most first. Each row has the picture, the type, a line of its models, units and IC.
- Only IC has cell bars, shares and the All equipment total.
- Units and IC keep their own coverage: a count the recording lacks for the country at that date reads Not recorded, never zero, and only recorded units say nothing was delivered.
- A row picks what the chart below stacks: that type's models. In IC, until a row is picked, the chart stacks every type: the largest six, then Other types.
- The chart's key with exact values follows the table.
- A picked day holds; Follow resumes playback. Pin binds the clicked native holder and date through aliases, excluding replacement holders.
- Parity builds prepare every ranked endpoint through the real reader after visible captures. Exact total and keyed amounts, including equipment-line subtotals where the combined amount is unknown, match the history view's canonical model. Ordinary builds request only visible endpoint rows and expose no model.

## Files

| File | Responsibility |
|---|---|
| [EquipmentProducedWidget.tsx](EquipmentProducedWidget.tsx), [ProducedDrillTable.tsx](ProducedDrillTable.tsx) | Caption line, race/lines, and the breakdown's table and key. |
| [PreparedEquipmentProducedWidget.tsx](PreparedEquipmentProducedWidget.tsx), [preparedProducedView.ts](preparedProducedView.ts), [usePreparedProduction.ts](usePreparedProduction.ts), [preparedProducedData.ts](preparedProducedData.ts) | Shared rendering, endpoint paging, preparation, cancellation and exact units/IC presentation. |
| [producedData.ts](producedData.ts), [producedView.ts](producedView.ts), [producedDrill.ts](producedDrill.ts) | Compressed exact values, ranking, captions, the default type, type rows and the breakdown chart; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [useProducedTypes.ts](useProducedTypes.ts), [equipmentProduced.css](equipmentProduced.css) | Options, the type list, registration and layout. |
