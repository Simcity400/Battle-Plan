# Stability & War support

## What it shows

Stability (solid) and war support (dashed) per country, with event markers and signed formula-part drills.

## Products it reads

[`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies values/formulas; [`stability-parts`](../../../../crates/dashboard/src/products/stability_parts.rs) supplies histories. Markers use [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs), [`idea-history`](../../../../crates/dashboard/src/products/idea_history.rs), [`party-history`](../../../../crates/dashboard/src/products/party_history.rs) and [`national-focus`](../../../../crates/dashboard/src/products/national_focus.rs). Country facts supply dated looks.

## Settings

Countries, campaign/follow/pin/range; line/marker toggles, end labels, fixed 0–100% or Zoom, playback/window and drill controls.

## Notable behaviour

- Values/formulas use native arithmetic and supplying reads; carried values say when read. Gaps and identity changes stop values and retained looks.
- Initial snapshots, gap boundaries and replacements are not events. Preserve observed bounds without inferred war declarers, founders or exact focus completions.
- Picks use drawn country/measure curves; overlap/key choices hold the date and disable Follow/Pin until selection. Scope/numeric changes invalidate picks; label refresh retains them.
- Drills keep signed parts, exact residuals and unknown totals within coverage/range/membership. Picking holds; Follow uses the aligned clock independently of parent time. Pin keeps country/measure and branch-relative day or Follow.
- [Shared drill bindings](../country-measure/drillCountry.ts) save native slot/ID/origin. Parts follow only its aliases; headers use the body date. Invalid holders refuse bodies; older missing-holder pins stay neutral.
- Mounted rows request flags; completed missing reads clear artwork. Keep full tooltip/event text accessible.

## Files

| File | Responsibility |
|---|---|
| [StabilityWidget.tsx](StabilityWidget.tsx), [stabilityData.ts](stabilityData.ts) | Values/formulas and chart. |
| [StabilityDrill.tsx](StabilityDrill.tsx), [stabilityDrillData.ts](stabilityDrillData.ts), [partsData.ts](partsData.ts) | Signed parts and histories. |
| [markers.ts](markers.ts) | Event rules; adjacent helper/render tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [stability.css](stability.css) | Options/registration/layout. |
