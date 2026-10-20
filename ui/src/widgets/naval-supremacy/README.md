# Naval supremacy

**What it shows:** Who controls each sea region day by day from one country's point of view, in the Air superiority timeline's form: the viewer and its friends, its enemies, or contested, by the game's own control rule.

**Product(s) it reads:** [`naval-supremacy`](../../../../crates/dashboard-naval-regions/README.md), a keyed family prepared from the daily naval supremacy reads with the diplomacy the control rule reads. Requests ask for daily runs (each day's value is its last reached recorded hour, never carried across midnight) for the chosen country, in 90-day blocks around the days shown.

**Settings:** One country (the dashboard's by default); Regions (all sea regions by default); Campaign, Follow, Pin or Range.

**Notable behaviour:**

- Control is the game's rule from the viewer's side (family before directed war, the quotient against the region's terrain minimum, friendly winning ties), not a share band. The supremacy share in the tooltip is the game's separate explanatory ratio, with its own numerator and denominator; the two are never mixed.
- Values are the recorded cached currents only; days are never reconstructed from estimates. A refused operand hatches that region's control while the known raw values stay in the tooltip. Days not recorded are shaded; hours still being prepared draw the shared band.
- Country view only: there is no per-side mode and no faction total.

**Files:** [NavalSupremacyWidget.tsx](NavalSupremacyWidget.tsx) draws the heat chart; [navalSupremacyData.ts](navalSupremacyData.ts) validates the reply and builds rows and tooltips; [useNavalSupremacy.ts](useNavalSupremacy.ts) requests the days.

Shared framework conventions: [widgets](../README.md).
