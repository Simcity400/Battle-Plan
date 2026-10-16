# Naval losses

**What it shows:** Ships and convoys lost or primarily credited as sunk by each country, in a race or lines, with a ships/convoys breakdown.

**Product(s) it reads:** [`naval-losses`](../../../../crates/dashboard-naval/src/products/naval_losses.rs) shares the retained-history [`sinkings`](../../../../crates/dashboard-naval/src/products/sinkings.rs) scan with [Ships sunk](../ships-sunk/README.md).

Prepared holder series supply exact reached windows and width summaries. Race
playback holds its current and adjacent windows; dense lines use shared chart
summaries and the breakdown samples exact component endpoints and extrema.
Late identity bridges add sparse signed corrections to historical supplying
dates. Hourly windows rerank corrected values; dense Top-N lines use the
min-rank bucket mask with corrected candidates. Hover, picks and keyboard dates
read exact values, with group and drill ranges bound to native holder intervals.
Unprepared ranges use the shared pending widget state, labelled **This range is
still being prepared.** Naval losses cannot sample a frame in isolation: retained
ship occurrences, convoy differences, baselines and late attribution corrections
depend on earlier observations. They remain pending rather than drawing a zero
or a count of the current retained vector.

**Settings:** Top-ten race of both kinds, Lost and Running total by default; ships/convoys/both, Sunk by, per-month counts, manual countries/smart groups, Show Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Running totals start at each requested table's first complete read; Both uses the later baseline. They are observed changes, not lifetime losses. Each part retains its own supplying date; late publications never fill earlier playback. Short breaks can recover; possible unseen retention pruning leaves affected totals/months unknown.
- Convoy buckets retain signed corrections and native wrap through widened endpoint differences. They identify neither individual convoy names nor exact event hours. Non-hostile causes and assists get no primary sunk-by credit.
- Tooltips show the selected time and exact counts. A single measure has one value row; Both keeps ships, convoys and their total.
- Native holders follow aliases without inheriting another country's counts. Prehistory ship identities use their first supplying read. Zero rows remain ranked; unknown counts stay unknown. Values outside exact JS integer range are refused.
- Both's drill requests the holder's full history independently of top N/groups. Overlapping line picks require a country choice. Exact signed rows/shares remain; only positive parts draw bars/ribbons.
- Picking a day holds; Follow uses actual clock/alignment; Pin stores branch-relative day and native selection origin. Restored holders must be proved at that origin, never retargeted through a reused tag. The standalone header follows the displayed holder/date. Client/selection changes revoke stale controls.

**Files:** [NavalLossesWidget.tsx](NavalLossesWidget.tsx) renders detail; `preparedNaval.ts` owns bounded windows, summaries and reached reads; `navalLossesData.ts` shapes exact counts; `navalPinOrigin.ts` restores identity origins; `NavalLossesCountryChip.tsx` supplies the standalone header. Adjacent tests cover data, paging, pins and interactions. Parity models are labelled **Naval losses exact values** and **Naval loss breakdown**. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).

The mounted Pin test runs the real prepared hook and shared window effects with
transport stubs. It checks holder restoration and bounded requests for unchanged
views, including repeated preparation progress with an immediate test wait.
