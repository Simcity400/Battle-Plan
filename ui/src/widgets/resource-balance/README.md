# Resource balance

## What it shows

Resource supply/demand and native surplus/shortfall as bars or history, with deal and pool-part drills; countries compare in separate panels.

## Products it reads

[`resource-balance`](../../../../crates/dashboard/src/products/resource_balance.rs) supplies independent pool/deal coverage and recorded resource definitions.

## Settings

Countries, campaign/follow/pin/range; bars/history, resource choices, order/scale, hide-empty, surplus shading, playback/axes and drill choices.

## Notable behaviour

- Native resource index distinguishes duplicate definitions; a token chip selects all its declarations. Names/icons and game order come from the content stack.
- Use the game’s net and export reserve, not actual purchases, for balance. Missing deal coverage leaves pools usable; discrepancies are Not from a listed deal only when deals are known.
- Signed corrections and native rounding residuals stay exact. Gaps and holder changes stop values; smart-group drills clip each native alias by that alias's membership without inventing zero.
- Hide empty removes only zero supply and demand, retaining signed shortages. Picks/overlap choices hold the drawn date. Pin saves resource token/ordinal, native holder origin and history extent; its header follows local picks/Follow. Older fixed-date pins use that date as origin; other missing origins stay unavailable. Follow uses the aligned clock independently of parent time.
- Every deal contributes; window only the displayed partner rows and date names/flags to the drill hour. Keep the interval index and same-request NumericReply through label refresh; mount visible panels/neighbours only.

## Files

| File | Responsibility |
|---|---|
| [ResourceBalanceWidget.tsx](ResourceBalanceWidget.tsx), [useResourceBalance.ts](useResourceBalance.ts) | Comparison panels, products and options. |
| [balanceData.ts](balanceData.ts) | Validation, pool meaning and exact history; adjacent tests. |
| [ResourceDealsView.tsx](ResourceDealsView.tsx), [resourceDeals.ts](resourceDeals.ts) | Signed deal drill and virtual partner rows. |
| [definition.ts](definition.ts), [index.ts](index.ts), [resourceBalance.css](resourceBalance.css) | Settings, registration and layout. |
