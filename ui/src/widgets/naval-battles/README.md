# Naval battles

**What it shows:** Recorded sea battles with fleets, native-side ship counts, losses and winners, as a log or biggest-battles table.

**Product(s) it reads:** [`naval-battles-manifest`, `naval-battles`, `naval-battles-ranking`](../../../../crates/dashboard/src/products/naval_battles.rs); [`naval-battle-details`](../../../../crates/dashboard/src/products/naval_battles/details.rs) enriches drawn rows; [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) names locations. Uses [shared battles](../battles/README.md).

**Settings:** All countries and Battle log by default; manual/smart groups, Biggest battles by Ships sunk/Peak ships/Length, top N and shared log/table controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Only full native NavalResultSources links join live combats to retained results. Province/hour/member similarity proves no identity; gaps, invalid transitions and transient merges retain refusals. Unlinked results keep roster, peak, length, fleet and ship identity unknown.
- Linked results replace reached live rows and retain the observed episode's roster/peak/span; later result reads do not lengthen it. Live disappearance without a result creates no ending. Prefetched windows cannot backfill future facts.
- Native selectors supply outcome. Convoys remain distinct from ships; detached terminal members make running losses/peaks minima. Missing classification stays unknown rather than borrowing counts.
- Drawn fleet details use full task-force-to-fleet references at the live roster's supplying read. Stored names require the recording language. Named losses join exact full combat/source references and wait for both event cutoff and supplying history read; duplicates remain distinct. No matched names does not prove no losses.
- Country filtering precedes ranking; incomplete country lists remain eligible. Names/flags retain endpoint dates and native identity boundaries. Shared code owns browsing, fitted tooltips and Jump.

**Files:** [NavalBattlesWidget.tsx](NavalBattlesWidget.tsx) supplies/enriches the adapter; `navalBattlesData.ts` validates/project rows and `navalBattleDetails.ts` shapes details; adjacent tests cover both. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
