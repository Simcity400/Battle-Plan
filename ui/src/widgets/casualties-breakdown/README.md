# Casualties breakdown

**What it shows:** One country's known losses grouped by native war episode then enemy, with a sunburst and exact signed recorded-loss details.

**Product(s) it reads:** [`casualty-breakdown`](../../../../crates/dashboard/src/products/war_casualties/breakdown.rs) shares the [`war_casualties.rs`](../../../../crates/dashboard/src/products/war_casualties.rs) scan. War grouping follows [`wars.rs`](../../../../crates/dashboard/src/products/wars.rs) and [Wars](../wars/README.md).

**Settings:** One country; war/enemy levels, shared sunburst Share/Fill and presentation, Sunburst/Recorded losses and inline/pop-out drill; Campaign, Follow or Pin.

**Notable behaviour:**

- Selection follows the native country holder through aliases. Missing or multiple viewpoint holders are refused, never replaced with the player. Hourly lifetime reads can expose losses before the country-summary sample updates.
- Each bilateral lifetime counts once. Ambiguous groups or missing grouping operands put its amount in **Other / not identified** with a reason. Ended relations keep their actual grouping/enemy supplying hours; replacement tags never name old losses.
- Unknown-attacker losses have no enemy and are excluded. No land/naval/air ring is inferred. Gaps can hide losses or wars; affected totals say **Recorded subtotal**.
- The signed total and all signed/zero lifetimes remain accessible; only positive amounts form slices, bands and shares. Fill is unknown for incomplete/missing final values or another holder.
- Picking a history day holds; Follow uses the actual clock independently of the sunburst. Pin preserves the explicit holder choice and branch-relative day or Follow. Outside-branch dates stay unavailable with a usable picker. Jump targets the supplying read.

**Files:** [CasualtiesBreakdownWidget.tsx](CasualtiesBreakdownWidget.tsx) loads block windows and renders; `casualtiesBreakdownData.ts` projects exact groups; `casualtyFlags.ts` reads dated detail flags. Adjacent tests cover these rules. `casualtiesBreakdown.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
