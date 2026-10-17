# Naval battles

**What it shows:** Recorded sea battles with fleets, native-side ship counts, losses and winners, as a log or biggest-battles table.

**Product(s) it reads:** [`naval-battles-manifest`, `naval-battles`, `naval-battles-ranking`](../../../../crates/dashboard-naval/src/products/naval_battles.rs); [`naval-battle-details`](../../../../crates/dashboard-naval/src/products/naval_battles/details.rs) enriches drawn rows; [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) names locations. Uses [shared battles](../battles/README.md).

**Settings:** All countries and Battle log by default; manual/smart groups, Biggest battles by Ships sunk/Peak ships/Length, top N and shared log/table controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Only full native NavalResultSources links join live combats to retained results. Province/hour/member similarity proves no identity; gaps, invalid transitions and transient merges retain refusals. Unlinked results keep roster, peak, length, fleet and ship identity unknown.
- Linked results replace reached live rows and retain the observed episode's roster/peak/span; later result reads do not lengthen it. Live disappearance without a result creates no ending. Prefetched windows cannot backfill future facts.
- Native selectors supply outcome. Convoys remain distinct from ships; detached terminal members make running losses/peaks minima. Missing classification stays unknown rather than borrowing counts.
- Drawn fleet details use full task-force-to-fleet references at the live roster's supplying read. Stored names require the recording language. Named losses join exact full combat/source references and wait for both event cutoff and supplying history read; duplicates remain distinct. No matched names does not prove no losses.
- Country filtering precedes ranking; incomplete country lists remain eligible. Names/flags retain endpoint dates and native identity boundaries. Shared code owns browsing, fitted tooltips and Jump.
- After normal visible captures, parity preparation retains every selected real history window and exposes complete ordered battle operands. Country/location labels and fleet details remain demanded only by the ordinary drawn rows. Bounded start, middle and end samples preserve the normal fleet descriptions, named losses, explicit unavailable values and rendered wording without preparing every offscreen detail key.

**Files:** [NavalBattlesWidget.tsx](NavalBattlesWidget.tsx) supplies/enriches the adapter; `navalBattlesData.ts` validates/project rows and `navalBattleDetails.ts` shapes details; adjacent tests cover both. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).

Naval logs, availability, rankings and drawn-row details read prepared pages.
Preparation shares the native census and retained sinking multiset
observer. Plain compressed JSON pages hold exact block replies and ranking records;
the shared catalogue supplies stable battle keys. Series events hold changed
fleet names and exact ship-read confirmations. Prepared named sinking
rows retain duplicates and both native and supplying dates. Drawn details read
only their battle, fleet and sinking pages; the production selection and detail
builder preserve full references and missing names. The Naval battles log and
biggest table retain their parity row labels.

A cold manifest may have no availability evidence and no refusal message while
its recorded hours are pending. The shared battle view renders Still being
prepared, then follows approximate committed pages or the exact newest proof.
