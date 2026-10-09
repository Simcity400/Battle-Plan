# Construction log

## What it shows

One row per observed construction project: building or repair, location, status, level bounds and assigned factories.

## Products it reads

[`construction-log`](../../../../crates/dashboard/src/products/construction_log.rs) supplies project histories and factory output. [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies dated state labels through the shared naming hook.

## Settings

All countries by default, or manual countries/groups; building types, new/repair projects, latest-change/start order, and shared log options. Campaign/follow/pin/range are available.

## Notable behaviour

- Rows appear at the read establishing them. A changed project re-enters at its latest change; Jump offers start/change/end dates.
- Level counts retain ranges and earlier-work lower bounds. Repair/build phases can share one project; gaps leave status and factories unread. Sampled speed does not bound work between reads.
- Finished requires a native completion outcome. Complete unlinked departures can be Removed; uncovered departures remain Left the queue. Building rises and factory output do not prove completion.
- Signed factory IC excludes bank withdrawals, instant progress and railway carry. Between-read projects without owner/target remain unidentifiable.
- Manual tags follow native holders/aliases; groups match at first read. Missing identity stays unknown. [useStateNamesAt](../../dashboard/useStateNamesAt.ts) resolves shown-hour states: older uncaptured fields use authored fallback; captured failures keep their reasons.

## Files

| File | Responsibility |
|---|---|
| [ConstructionLogWidget.tsx](ConstructionLogWidget.tsx), [useConstructionLog.ts](useConstructionLog.ts) | Log rendering, products, filters and visible-row names. |
| [constructionData.ts](constructionData.ts) | Validation, project bounds, status and row wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [constructionLog.css](constructionLog.css) | Registration, settings and layout. |
