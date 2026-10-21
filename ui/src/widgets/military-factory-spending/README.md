# Military factory spending

## What it shows

Where one country's military factory IC goes for air or land equipment: new equipment and conversions, each by equipment type and design. Breakdown is the shared Sunburst; By month draws one stacked column per calendar month on the time axis. It shares Dockyard spending's bodies ([PurposeSpending.tsx](../dockyard-spending/PurposeSpending.tsx)) and its purpose split ([dockyardData.ts](../dockyard-spending/dockyardData.ts)).

## Products it reads

[`military-factory-spending`](../../../../crates/dashboard-base/src/products/military_factory_spending.rs) answers both views through prepared queries.

- New and Converted are Production spending's exact applied input of the chosen equipment's designs, split by the purpose the recorder took at each production tick. Converted is the input of military production while its line converted stockpiled equipment: conversion progress, including ticks that completed nothing, not a count of source units removed. It belongs to the target design; the source design is not recorded.
- The By month view reads the same input from production's shared spent series: a month's amount is its end's cumulative input less the hour before its start, inside one uninterrupted observation.

## Settings

Equipment (Air, Land), View (Breakdown, By month; also switched in the header), the three ring levels, IC decimals and the shared sunburst options. The catalogue lists Air factory spending by month, Land factory spending and Land factory spending by month beside the widget's own card (Air, Breakdown). Campaign/follow/pin/range time; one country.

## Notable behaviour

- A design whose type content cannot resolve could be the chosen equipment. It is left out, and the purpose it belongs to is marked incompletely attributed (hatched); By month hatches that month's bands and has no known total.
- Input of a tick whose purpose the recorder could not classify is its own Purpose not recorded part (a ring part and a By month band), by type and design, never New's amount. Its evidence is the summed magnitude of that input, so corrections never cancel it. Where such input joined the dates shown (or a month), the part is hatched down its subtree and New and Converted are marked incompletely attributed, however small the net amount; the whole stays known.
- Input recorded with a purpose military production never has (a ship refit) stays in the whole as an Other purposes part and band, and the reply warns about it.
- Equipment spending the source cannot read is not zero: New and Converted each hold an unknown part.
- A month counts only where the equipment source was read and untainted: before its first read, after a recording gap or missed publication taints it, or after its observation restarts, its amounts are not known.
- Before preparation reaches the dates shown, the reply answers its prepared prefix as Production spending's does; By month bands the pending months.

## Files

| File | Responsibility |
|---|---|
| [MilitaryFactorySpendingWidget.tsx](MilitaryFactorySpendingWidget.tsx) | The widget's product, purposes and month bands for the shared bodies. |
| [factoryData.ts](factoryData.ts) | Validation, sunburst items and month columns; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Equipment, views, levels, shared options, catalogue cards and registration. |
