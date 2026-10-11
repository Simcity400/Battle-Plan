# Political power spending

## What it shows

One dated log row per recorded political-power payment, with category chips and optional running totals.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) through [usePoliticsSpending](../../dashboard/usePoliticsSpending.ts). The [shared spending kit](../../dashboard/politicsSpending.ts) owns projection and wording.

## Settings

Countries, campaign/follow/pin/range and shared [spending options](../../dashboard/politicsSpendingOptions.ts): categories, order, artwork, totals and log presentation. Follow is the default.

## Notable behaviour

- Exact evaluated charges and differing realized changes remain distinct. A batch appears at its supplying read; row countries/targets and flags use the native action hour.
- The overall total includes every category; each category total appears only for a shown category. An empty category selection shows them all. Unrecorded spans produce gap rows and an unknown campaign total. Grants, refunds, resets and clamps are excluded by the Rust spending projection.
- Shared virtual log rows retain pending/unavailable flag reasons. Native coverage gaps use the gap-start country; membership gaps are flagless. Jump uses the dated row action.

## Files

| File | Responsibility |
|---|---|
| [PoliticalPowerSpendingWidget.tsx](PoliticalPowerSpendingWidget.tsx) | PP adapter for shared PoliticsSpendingLog. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Metadata, shared settings and category choices. |
