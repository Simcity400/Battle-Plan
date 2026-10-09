# Territory transfers

**What it shows:** States flowing from losers to winners in captured or derived peace outcomes, as a two-column flow diagram with complete transfer details.

**Product(s) it reads:** [`peace-deals`](../../../../crates/dashboard/src/products/peace_deals.rs) supplies conference/campaign transfer graphs; [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies dated state facts. Attribution is owned by the product.

**Settings:** Whole world; no country picker. State count (default)/victory points, whole campaign/one conference, giver/taker ribbon colour, Transfer diagram/Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Endpoints/pins use native slot/ID pairs through renames, separating replacements. Occurrences retain dated endpoint tags/reads for looks; missing event-hour identities remain counted.
- Native event dates, supplying observations and VP reads independently gate visibility. Range includes both endpoints; Pin is cumulative through its moment. Repeated transfers of a state are distinct occurrences. Backward seeks remove future ribbons immediately.
- Conference choices/pins bind campaign/branch. Automatic picks the newest reached conference; missing/future choices stay explicit. Captured, derived, grouped war-end and unknown attribution stay distinct; gaps/failures cannot prove absence.
- Excess Canvas nodes fold into Other, retaining raw links and signed/zero/unknown details. Names/flags use occurrence dates; colours use all graphs before filtering/fitting.
- VP width requires every occurrence's dated VP value. Missing inputs or native ordered checked-sum overflow make the whole link unknown; zero/negative totals have no positive width. No partial sum becomes a known amount.
- Picks retain raw contributors; a new cohort resets state detail. Sparse history keeps read gating. Picking holds; Follow uses playback; Pin saves scope/contributors/day and range start. Other branches cannot restore it.

**Files:** [TerritoryTransfersWidget.tsx](TerritoryTransfersWidget.tsx) renders; `territoryData.ts` shapes graphs; `territoryTooltip.ts` preserves facts; `transferDrillData.ts`/`TransferDrill.tsx` own details; `useConferenceChoices.ts` supplies choices, with adjacent tests. CSS, `definition.ts` and `index.ts` own style/options/registration; [flow kit](../../charts/flow/README.md) owns drawing/navigation.

Shared framework conventions: [widgets](../README.md).
