# Political power spending breakdown

## What it shows

The same PP payments as the spending log, grouped as category → specific item in a sunburst.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) through [usePoliticsSpending](../../dashboard/usePoliticsSpending.ts); [politicsSpending.ts](../../dashboard/politicsSpending.ts) owns exact grouping.

## Settings

Countries, campaign/follow/pin/range, category/item level toggles and shared sunburst options. Whole campaign is the default.

## Notable behaviour

- Amounts are evaluated charges, with realized changes retained separately. Live includes only supplied reads; campaign uses branch end and range uses its counting window.
- Hours not recorded is separate from Not identified spending. Gaps keep full totals unknown and prevent Fill using a partial denominator; the recorded subtotal remains exact.
- Law/advisor item tooltips retain the grouped summary plus game-style effects/targets at the most recent matched action. They state action and read dates; categories and folded Other keep the shared summary.

## Files

| File | Responsibility |
|---|---|
| [PoliticalPowerSpendingBreakdownWidget.tsx](PoliticalPowerSpendingBreakdownWidget.tsx) | Shared spending-to-Sunburst adapter; adjacent renderer test. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Levels, common options and registration. |
