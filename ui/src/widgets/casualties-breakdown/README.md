# Casualties breakdown

**What it shows:** One country's known losses grouped by native war episode then enemy, with a sunburst and exact signed recorded-loss details.

**Product(s) it reads:** Prepared [`casualty-breakdown`](../../../../crates/dashboard-wars/src/products/war_casualties/breakdown/prepared.rs) joins the shared unfiltered casualty block records with [`wars::prepared::projection`](../../../../crates/dashboard-wars/src/products/wars/prepared.rs). The existing breakdown projection and reply builder select native holders and attribute losses; the existing Wars walk supplies viewpoint grouping and dated names. No widget request scans replay history.

**Settings:** One country; war/enemy levels, shared sunburst Share/Fill and presentation, Sunburst/Recorded losses and inline/pop-out drill; Campaign, Follow or Pin.

**Notable behaviour:**

- Selection follows the native country holder through aliases. Missing or multiple viewpoint holders are refused, never replaced with the player. Hourly lifetime reads can expose losses before the country-summary sample updates.
- Each bilateral lifetime counts once. Ambiguous groups or missing grouping operands put its amount in **Other**. Ended relations keep their actual grouping/enemy supplying hours; replacement tags never name old losses.
- An open lifetime's original supplying boundary survives block publication. Branch-wide endpoint presence and uncertainty changes preserve earlier supplying boundaries and holder-overlap refusals outside the requested window.
- Enemy titles and table sorting use the dated localized country name. Missing names remain **Country name unavailable**, with the recorded enemy key in details; empty known names remain empty.
- Content changes reload the prepared windows and their dated war names together.
- A window preparation has not reached shows the prepared prefix's newest window at its last hour, exactly as that window gives it, with the note "Losses through <date>; the rest is still being prepared.", replaced as the prefix grows; Fill waits for the final window's exact values. Before any loss is observed, the widget reads "This range is still being prepared."
- Unknown-attacker losses have no enemy and are excluded. No land/naval/air ring is inferred. Gaps can hide losses or wars; affected totals say **Recorded subtotal**.
- The signed total and all signed/zero lifetimes remain accessible; only positive amounts form slices, bands and shares. Fill is unknown for incomplete/missing final values or another holder.
- Drill totals retain the same prepared-through date and preparation note in inline, pop-out and pinned views.
- Picking a history day holds; Follow uses the actual clock independently of the sunburst. Pin preserves the explicit holder choice and branch-relative day or Follow. Outside-branch dates stay unavailable with a usable picker. Jump targets the supplying read.

**Files:** [CasualtiesBreakdownWidget.tsx](CasualtiesBreakdownWidget.tsx) loads block windows and renders; `casualtiesBreakdownData.ts` projects exact groups; `casualtyFlags.ts` reads dated detail flags. Adjacent tests cover these rules. `casualtiesBreakdown.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).

Parity builds expose "Casualty breakdown exact values" with each signed lifetime,
native holder, supplying/grouping/enemy dates, war names, coverage and window.
