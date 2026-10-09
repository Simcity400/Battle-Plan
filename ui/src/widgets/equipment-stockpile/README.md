# Equipment stockpile

## What it shows

Stock, weekly production and the game’s surplus/shortage, as bars or history; countries compare in separate panels.

## Products it reads

[`equipment-stockpile`](../../../../crates/dashboard/src/products/equipment_stockpile.rs) supplies values. `stockpile-artwork` in [artwork.rs](../../../../crates/dashboard/src/products/equipment_stockpile/artwork.rs) supplies dated best-variant pictures.

## Settings

Countries, campaign/follow/pin/range; type/variant/arm grouping, included types, separately saved history choices, bar order/scale, playback/axes and stock/balance drills.

## Notable behaviour

- The game computes balance only for non-AI-controlled countries. Otherwise, and for variants, show stock. Need is not apportioned to designs. Unknown, zero, negative or saturated need supplies no percentage.
- Derive need from the availability operand at the balance write; never use current supply values. Older ordinary types retain their stock-minus-balance derivation; missing special-type availability stays unknown.
- Balance, stock and weekly output keep separate reads/coverage. Gaps/holder changes end carries. Unsupported rates or unresolved nonzero/unknown classifications leave affected sums unknown.
- Designs use read-dated names and campaign/design witnesses for saved choices. Type art uses the native best-candidate rule, not largest stock; missing operands/art keep placeholders and reasons.
- Drills use balance-write contributions and wrapping/residual parts. Signed/zero rows remain selectable. Picking holds; Follow uses the clock. Pin saves the original native holder, semantic equipment and time. Aliases exclude tag reuse; headers use the body date. Missing origins leave neutral headers/unavailable bodies.
- Keep numeric pick ownership through content-only refresh and mount only visible comparison panels and neighbours.

## Files

| File | Responsibility |
|---|---|
| [EquipmentStockpileWidget.tsx](EquipmentStockpileWidget.tsx), [useEquipmentStockpile.ts](useEquipmentStockpile.ts) | Panels, products and options. |
| [StockpileDrill.tsx](StockpileDrill.tsx), [stockpileDrill.ts](stockpileDrill.ts), [stockpileData.ts](stockpileData.ts) | Exact grouping, design witnesses and drills; adjacent tests. |
| [useStockpileArtwork.ts](useStockpileArtwork.ts) | Dated artwork. |
| [definition.ts](definition.ts), [index.ts](index.ts), [equipmentStockpile.css](equipmentStockpile.css) | Options, registration and layout. |
