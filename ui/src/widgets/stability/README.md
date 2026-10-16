# Stability & War support

## What it shows

Stability (solid) and war support (dashed) per country, with event markers and signed formula-part drills.

## Products it reads

Prepared series supply width-bounded stability and war-support geometry, exact reached values and supplying-read dates. [`country-stability`](../../../../crates/dashboard-stability/src/products/country_stability.rs) supplies at-hour formulas and the history reference; [`stability-parts`](../../../../crates/dashboard-stability/src/products/stability_parts.rs) shares the preparation scan and supplies the history reference. Prepared part runs, observed coverage and absolute contribution summaries supply the drill. Markers use [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs), [`idea-history`](../../../../crates/dashboard-base/src/products/idea_history.rs), [`party-history`](../../../../crates/dashboard-party-history/src/products/party_history.rs) and [`national-focus`](../../../../crates/dashboard/src/products/national_focus.rs). Country facts supply dated looks.

Ruling-party markers use the shared prepared party change events. Exact windows
keep individual legacy badges; dense windows use membership-filtered clusters
with exact counts and reached event pages. The "Stability party markers" parity
model compares the same dated changes on both paths.

## Settings

Countries, campaign/follow/pin/range; line/marker toggles, end labels, fixed 0–100% or Zoom, playback/window and drill controls.

## Notable behaviour

- Values/formulas use native arithmetic and supplying reads; carried values show an As of date or the range of dates across countries. Gaps and identity changes stop values and retained looks.
- Initial snapshots, gap boundaries and replacements are not events. Preserve observed bounds without inferred war declarers, founders or exact focus completions.
- Picks use drawn country/measure curves; overlap/key choices hold the date and disable Follow/Pin until selection. Scope/numeric changes invalidate picks; label refresh retains them.
- The parts palette ranks exact absolute contribution over the whole selected range. Hourly windows use the same signed bands as the history view; dense geometry summarises each sparse part's ordered extrema independently, so a missing child bucket cannot hide known contributions. Observed coverage controls the signed stack. Selected and hovered values stay exact.
- Parts beyond prepared coverage sample the recording directly. Wide tails carry approximate spans through the shared stack and drawing helpers, with one Approximate key; views touching at most thirteen unprepared blocks read every frame. Formula terms and non-character lines work for countries and part keys first seen in the tail. Carried advisor/leader identities need the earlier supplying frame's character names; their exact breakdown stays pending until prepared, never a partial total.
- Drills keep signed parts, exact residuals and unknown totals within coverage/range/membership. Picking holds; Follow uses the aligned clock independently of parent time. Pin keeps country/measure and branch-relative day or Follow.
- Source names preserve localized and literal empty text. Unnamed formula terms retain exact values with a neutral unavailable title and their key in hover details.
- [Shared drill bindings](../country-measure/drillCountry.ts) save native slot/ID/origin. Parts follow only its aliases; headers use the body date. Invalid holders refuse bodies; older missing-holder pins stay neutral.
- Mounted rows request flags; completed missing reads clear artwork. Keep full tooltip/event text accessible.
- Focus markers share the text-only dated focus lookup. Country-dependent titles resolve at the completion's observed hour; static localized text appears while that read loads.

## Files

| File | Responsibility |
|---|---|
| [StabilityWidget.tsx](StabilityWidget.tsx), [stabilityData.ts](stabilityData.ts) | Values/formulas and chart. |
| [StabilityDrill.tsx](StabilityDrill.tsx), [stabilityDrillData.ts](stabilityDrillData.ts), [partsData.ts](partsData.ts) | Signed parts and histories. |
| [preparedParts.ts](preparedParts.ts) | Windowed part geometry, exact reached values, whole-range palette totals and the "Stability part values" parity model. |
| [preparedStability.ts](preparedStability.ts), [usePreparedScalarParity.ts](../country-measure/usePreparedScalarParity.ts) | Windowed prepared values and the "Stability chart values" browser parity model. |
| [markers.ts](markers.ts) | Event rules; adjacent helper/render tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [stability.css](stability.css) | Options/registration/layout. |
