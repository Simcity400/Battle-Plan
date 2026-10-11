# Naval losses

**What it shows:** Ships and convoys lost or primarily credited as sunk by each country, in a race or lines, with a ships/convoys breakdown.

**Product(s) it reads:** [`naval-losses`](../../../../crates/dashboard/src/products/naval_losses.rs) shares the retained-history [`sinkings`](../../../../crates/dashboard/src/products/sinkings.rs) scan with [Ships sunk](../ships-sunk/README.md).

**Settings:** Top-ten race of both kinds, Lost and Running total by default; ships/convoys/both, Sunk by, per-month counts, manual countries/smart groups, Show Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Running totals start at each requested table's first complete read; Both uses the later baseline. They are observed changes, not lifetime losses. Each part retains its own supplying date; late publications never fill earlier playback. Short breaks can recover; possible unseen retention pruning leaves affected totals/months unknown.
- Convoy buckets retain signed corrections and native wrap through widened endpoint differences. They identify neither individual convoy names nor exact event hours. Non-hostile causes and assists get no primary sunk-by credit.
- Tooltips show the selected time and exact counts. A single measure has one value row; Both keeps ships, convoys and their total.
- Native holders follow aliases without inheriting another country's counts. Prehistory ship identities use their first supplying read. Zero rows remain ranked; unknown counts stay unknown. Values outside exact JS integer range are refused.
- Both's drill requests the holder's full history independently of top N/groups. Overlapping line picks require a country choice. Exact signed rows/shares remain; only positive parts draw bars/ribbons.
- Picking a day holds; Follow uses actual clock/alignment; Pin stores branch-relative day and native selection origin. Restored holders must be proved at that origin, never retargeted through a reused tag. The standalone header follows the displayed holder/date. Client/selection changes revoke stale controls.

**Files:** [NavalLossesWidget.tsx](NavalLossesWidget.tsx) renders/loads detail; `navalLossesData.ts` shapes exact counts; `navalPinOrigin.ts` restores identity origins; `NavalLossesCountryChip.tsx` supplies the standalone header. Adjacent tests cover data/pins/interactions. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
