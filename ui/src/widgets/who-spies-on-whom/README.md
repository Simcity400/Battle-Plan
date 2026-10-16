# Who spies on whom

## What it shows

Directed operative-placement counts between acting and target countries, with named operative inventories in tooltips.

## Products it reads

[`operative-placements`](../../../../crates/dashboard/src/products/operative_placements.rs) supplies sparse pair/roster histories; [`country-styles`](../../../../crates/dashboard-base/src/products/country_styles.rs) and shared country flags/names supply dated looks.

## Settings

All/manual/group countries, campaign/follow/pin; Most countries shown ranks sent + received and folds the rest into Other countries.

## Notable behaviour

- Active mission assignments use the mission target; active operation assignments use the operative’s own operation reference. A reserved operation never replaces a current mission target. Recruitment pools and ineligible/captured/departed assignments are excluded.
- Count each operative identity once. Complete membership reads can hold through consecutive unread hours with the original date; gaps, failed/truncated/untimed walks and identity changes end the hold. Known empty is distinct from missing capture.
- Manual picks touch either endpoint and follow native aliases, excluding unrelated reuse. Fixed colours use the full ordered holder catalogue, independent of visible pairs.
- Other folds losslessly, including self flows; exact arc totals and full operative inventories remain available. Unknown looks retain diagnostic reasons. Jump uses the supplying membership read.
- Prepared day windows retain the actual left-edge membership read. Parity builds expose counts and inventories as "Operative placement values".

## Files

| File | Responsibility |
|---|---|
| [WhoSpiesOnWhomWidget.tsx](WhoSpiesOnWhomWidget.tsx) | Shared chord chart and dated country looks. |
| [placementsData.ts](placementsData.ts) | Validation, holder filtering, folding and tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [placements.css](placements.css) | Options, registration and layout. |
