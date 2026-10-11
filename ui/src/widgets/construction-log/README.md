# Construction log

## What it shows

One row per observed construction project: building or repair, location, status, level bounds and assigned factories.

## Products it reads

[`construction-log-manifest` and `construction-log`](../../../../crates/dashboard/src/products/construction_log.rs) supply the branch's windows and building types, then each window's project histories and factory output. [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies dated state labels through the shared naming hook.

## Settings

All countries by default, or manual countries/groups; building types, new/repair projects, latest-change/start order, and shared log options. Campaign/follow/pin/range are available.

## Notable behaviour

- Rows appear at the read establishing them. A changed project re-enters at its latest change; Jump offers start/change/end dates.
- Level counts retain ranges and earlier-work lower bounds. Repair/build phases can share one project; gaps leave status and factories unread. Sampled speed does not bound work between reads.
- Finished requires a native completion outcome. Complete unlinked departures can be Removed; uncovered departures remain Left the queue. Building rises and factory output do not prove completion.
- Signed factory IC excludes bank withdrawals, instant progress and railway carry. Between-read projects without owner/target remain unidentifiable.
- Building and state titles preserve successfully localised empty text and titles equal to their keys. Missing titles stay unavailable; diagnostic details retain their keys.
- Manual tags follow native holders/aliases; groups match at first read. Missing identity stays unknown. [useStateNamesAt](../../dashboard/useStateNamesAt.ts) resolves shown-hour states: older uncaptured fields use authored fallback; captured failures keep their reasons.
- One window per replay block lists every project with a row date in it. The widget holds the shown hour's window, the next during Live and, back from there, as many rows below the drawn ones as are drawn; short of them it reaches further back. Rows dated before the oldest window held unbroken wait for it, so the list is always the whole log's newest rows. A far jump keeps the last shown hour until its window arrives.

## Files

| File | Responsibility |
|---|---|
| [ConstructionLogWidget.tsx](ConstructionLogWidget.tsx), [useConstructionLog.ts](useConstructionLog.ts) | Log rendering, filters and visible-row names; products and the windows held. |
| [constructionData.ts](constructionData.ts) | Validation, window merging and reach, project bounds, status and row wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [constructionLog.css](constructionLog.css) | Registration, settings and layout. |
