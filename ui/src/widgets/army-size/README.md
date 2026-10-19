# Army size

**What it shows:** Each country's land army as a race, lines, battalion mix or full breakdown.

**Product(s) it reads:** [`army-size`](../../../../crates/dashboard-forces/src/products/army_size.rs) supplies capabilities and recording-block histories; its inventory/composition modules own native attribution and arithmetic.

**Settings:** Top-ten race by default; manual countries and smart groups; manpower, divisions, current HP or battalions; Owner/Host attribution; authored-group parts, a View selector for race, lines, battalion mix or breakdown, and inline/pop-out drill. Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Automatic measure is manpower when recorded, otherwise divisions. An unavailable saved measure stays saved and labelled while the chart uses the available default. Mix always counts battalions, including support companies; divisions and HP stay unsplit.
- Battalion groups retain their authored content identities, never a division representative or guessed category. Their titles resolve the exact authored localization key; missing titles say unavailable and report the key in diagnostics. Unresolved groups retain counted sub-unit identities, distinct from an unidentified numeric remainder. Null sub-units count no battalion.
- Owner includes expeditionary senders; Host is the roster country. Manpower contributors do not change that attribution. Exact raw integers supply amounts/labels; floating values supply geometry. Missing inputs break only the affected measures; empty complete rosters prove zero.
- Windows follow replay blocks. Race needs the moment and prefetch; lines/mix/drill load the viewport and margin. Unloaded windows stay gaps, and top-N membership is dated. Capitulation markers do not end values.
- Prepared clients read accepted army windows and capabilities, then draw uncovered lines from approximate keyframe summaries immediately. Zooms touching at most thirteen unprepared blocks and reached block windows read exact frames. Battalion windows reuse the native composition builder. Dense lines use width summaries; exact short views retain their normal drawing path. Top selection uses prepared positions in the prefix and the native ranking sweep over transient tail values. Hover, picks and navigation use exact values.
- Drill keeps the native holder's aliases/full history beyond ranks/groups. Scalar measures use **Other**. Picking holds; Follow uses the aligned clock. Pin keeps measure, attribution, origin and branch-relative day; headers use the body date or stay neutral.
- Invalid origins refuse the body without selecting a current actor. Parent states/actor choosers keep neutral dated headers.

**Files:** [ArmySizeWidget.tsx](ArmySizeWidget.tsx) loads and renders; `ArmyBreakdown.tsx`/`ArmyCountryChip.tsx` own details/header. [armySizeData.ts](armySizeData.ts) validates/windows data; `armyDrill.ts` shapes exact parts and pins; adjacent tests cover these rules. `armySize.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).

`country-people` adds reached recruitment, mobilisation, deployed branch crews and state pools to the manpower breakdown. Details follow the same country and hour; each sliced operand preserves its supplying date.
The existing breakdown includes a plain Division templates section at its selected country/hour: every used and unused template with layout/settings/restrictions/naming groups/roles/caches, then country staff/garrison and template-bound HQ context. The exact-hour `army-size` request uses roster ownership rather than deployed membership; a country with zero deployed divisions still has these sections. Requests and responses retain source/content/date identity. No widget is added.
