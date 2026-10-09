# Casualties

**What it shows:** Each country's cumulative casualties dealt/taken as a butterfly chart, or a race of losses taken. These war-relation counters cover land, sea and air together.

**Product(s) it reads:** [`war-casualties`](../../../../crates/dashboard/src/products/war_casualties.rs) supplies exact hourly country totals, identities and membership. Native loss and unknown-recovery evidence is beside that code.

**Settings:** Top ten by dealt + known taken by default; manual countries and smart groups; Dealt vs taken/Race of losses taken, include losses with no recorded attacker (off), butterfly values/ratio. Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Taken sums the victim's own counters; dealt sums opponents' losses. A range shows cumulative totals at its last hour, not window losses. Holding/clipping creates no new read; gaps never become zero. No relations means no row; recorded zero stays zero.
- Unknown-attacker losses add only to taken when exact recovery is possible. They are not an attrition category. Recovery needs a zero baseline and agreeing increments on spanning relations; gaps, resets, disagreements or loss of continuity make the lifetime total unavailable. Neither sum nor maximum of raw relation counters recovers it.
- Top N always uses dealt + known taken; unknown losses affect display only. Signed corrections stay exact/selectable; only positive bars draw. Incomplete totals, corrections and zero/unknown taken suppress ratios.
- Values/ranks use every hourly read; equal values/repeated deltas compress without losing hours or supplying dates. Missing hours stay gaps; oversized complete replies refuse the shared byte budget.
- Ended/disappeared relations retain counters. Missing history can omit positive losses or negative corrections: **Recorded subtotal** (legacy `atLeast` flags) is not a lower bound. Entire unseen wars and final ticks between reads cannot be recovered.
- Manual tags follow their native holders/aliases without inheriting another holder's counters. Looks and Jump use the supplying sample date; no name or flag crosses an identity gap.

**Files:** [CasualtiesWidget.tsx](CasualtiesWidget.tsx) renders the shared butterfly/race; [casualtiesData.ts](casualtiesData.ts) validates and indexes exact BigInt histories, with adjacent tests. `casualties.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
