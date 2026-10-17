# Equipment stockpile

## What it shows

One country's equipment, type by type, led by the game's surplus or shortage. Each type shows the models held or being made, and what its production lines make a day. Countries compare in separate panels. Over time draws chosen types as lines. A type's breakdown shows its models.

## Products it reads

- [`equipment-stockpile`](../../../../crates/dashboard-equipment-stockpile/src/products/equipment_stockpile.rs) supplies:
  - stock by model;
  - each model's output per day: the game's per-day formula, each line truncated, then summed;
  - the IC a day its lines put in, and their lowest and highest cost per unit;
  - the logistics record (balance, its stock and the available operand).
- `stockpile-artwork` in [artwork.rs](../../../../crates/dashboard-equipment-stockpile/src/products/equipment_stockpile/artwork.rs) supplies dated best-variant pictures.

The prepared stockpile adapter supplies one reached date, the visible chart
summary, independent naming pages and next-change navigation. Native holder and
cohort spans constrain the query. Type/domain summaries use correlated group
operands, including corrections for late names and changed tokens. Explicit type
filters merge selected type changes. Exact views use the same plotted runs as the
recorded product, including supplying bounds at the viewport edges. Dense ratios
divide exact numerator/denominator samples in the same float order; hover amounts
come from dated canonical runs. The recorded product remains the parity oracle.
Before preparation completes, the table at the shown hour comes from the
recording: each last stock, line and logistics read carries to the hour, so its
amounts are exact. Its type groups classify each design by its latest name by
that hour, and charts and automatic choices wait for preparation.

## Settings

- Countries, and campaign/follow/pin/range.
- View: Table or Over time.
- Count, for the table: Units or IC. IC changes only the output column, because the game gives stock no IC value.
- Included types.
- For Over time: the types drawn, chosen with the compact picker on its caption line, plus playback and axes.
- Breakdown placement.

## Notable behaviour

**What the surplus is:**
- The surplus or shortage is the game's Logistics figure, available minus needed.
- Needed is derived from the availability operand at the balance write, never from current supply values. Older ordinary types keep their stock-minus-balance derivation; missing special-type availability stays unknown.
- Available is the stock the daily update counted, except for convoys (the convoy pool) and supply trucks and trains (the supply system's term is added).
- So stock appears only where it adds something:
  - on a models line: each model's count, before its name, when a type has several models; a lone model's stock where it is not the surplus ("He 111 · 0 in stock"); and "71 in stock, 56 available" where availability differs, for types content marks;
  - in the tooltip;
  - in the breakdown.

**The table:**
- It lists only types with anything at the date (stock, output, need or surplus, known or not). The rest are counted on a muted line.
- It sorts shortages first for a player-controlled country.
- The game computes no surplus for AI-controlled countries; there the table leads with what is in stock.

**Numbers:**
- Rates per day use `formatRate`: whole from 10, two significant figures below.
- Stock, surplus and need are whole units.
- A bridge from before the per-day output shows output as not recorded.

**Unknown values:**
- Balance, stock and output keep separate reads and coverage. Gaps and holder changes end carries.
- Unsupported rates, or unresolved nonzero/unknown classifications, leave affected sums unknown.

**Names and pictures:**
- Models use read-dated names, and saved choices use campaign/design witnesses.
- Type art uses the native best-candidate rule; missing artwork uses a hammer fallback.
- Equipment titles preserve authored empty text and titles equal to their keys.

**The breakdown:**
- Model bars are each model's share of its type's stock: one unit, so they compare.
- It shows the logistics equation, and per model the output a day, the IC a day and the line cost per unit. A model held but not in production has no recorded cost.
- Below is stock over time by model, with needed dashed.
- Picking holds; Follow uses the clock. Pin saves the original native holder, type and time.
- A breakdown saved by variant (with its design) or by arm keeps that grouping and opens on what it names.
- Keep numeric pick ownership through content-only refresh, and mount only visible comparison panels and their neighbours.

## Files

| File | Responsibility |
|---|---|
| [EquipmentStockpileWidget.tsx](EquipmentStockpileWidget.tsx), [StockpileTable.tsx](StockpileTable.tsx), [useEquipmentStockpile.ts](useEquipmentStockpile.ts) | Panels, the table, Over time, products and options. |
| [stockpileView.ts](stockpileView.ts), [stockpileData.ts](stockpileData.ts), [StockpileDrill.tsx](StockpileDrill.tsx), [stockpileDrill.ts](stockpileDrill.ts) | Table rows, exact grouping, design witnesses and the breakdown; adjacent tests. |
| [useStockpileArtwork.ts](useStockpileArtwork.ts) | Dated artwork. |
| [preparedStockpileChart.ts](preparedStockpileChart.ts), [usePreparedStockpileChart.ts](usePreparedStockpileChart.ts) | Integer summaries, exact supplying edges, owner-scoped display and hover queries. |
| [stockpileParityModel.ts](stockpileParityModel.ts) | Complete exact displayed stockpile and breakdown facts for the parity build. |
| [definition.ts](definition.ts), [index.ts](index.ts), [equipmentStockpile.css](equipmentStockpile.css) | Options, registration and layout. |

A table before complete preparation labels its equipment types Approximate.
Its supplying-read quantities remain exact; a later nonempty token can revise
which type holds a model. This note disappears with the complete reply.
