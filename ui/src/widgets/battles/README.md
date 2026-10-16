# Shared battle components

**What it shows:** The shared BattleLogView used by [Land battles](../land-battles/README.md) and [Naval battles](../naval-battles/README.md). It provides log/leaderboard browsing; this folder is not a separately registered widget.

**Product(s) it reads:** Caller-selected manifest, window and ranking products from [`land_battles.rs`](../../../../crates/dashboard/src/products/land_battles.rs) or [`naval_battles.rs`](../../../../crates/dashboard/src/products/naval_battles.rs), plus [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs). Shared wire types live in [`battles.rs`](../../../../crates/dashboard-base/src/products/battles.rs).

**Settings:** `battleOptions` supplies Battle log/Biggest battles, caller-defined measure choices, user-selected top N, log/table controls and sorting. The owning widget declares campaign/time/country settings; header sorting stays within the server-selected leaderboard. Selected measure follows the current ranking measure; explicit measure sorts stay fixed. Background loading shows one Updating status.

**Notable behaviour:**

- BattleViewAdapter supplies validators, row lookups, measures, labels, loss formatting, details and diagnostics. An adapter may supply its own `view` of the same rows and props, with the source, track, shown hour and place-name state; it then also draws the empty list. Shared code never infers winners or domain duration conventions.
- Log windows follow recording blocks and drawn rows. Ranking filters countries before top N; logs filter locally. Each country matches at its own supplying hour, following native holders through aliases. Unknown participant lists must survive filters; pending identities stay loading.
- Null start means unrecorded; null end means ongoing; an unknown end is distinct. **By** dates remain bounds. Logs sort endings by observation hour; Jump uses start, else place hour, never an absent ending read. Live pins ongoing rows.
- Place naming, country facts and count/loss reads have separate dates. Details and looks are requested only for drawn rows. Place names already read stay while the next drawn rows load; a name still loading reads **Loading…**, never the unnamed label. Compact flags and fitted tooltips retain complete accessible lists.
- Unavailable participant flags retain the shared placeholder border and rounded corners regardless of stylesheet load order.
- Measures use unique string labels across each side's counts/losses; ambiguous labels refuse a row, and an unknown side keeps a sum unknown. Values may be fractional; row IDs preserve focus during motion.
- After normal visible captures, parity builds retain every manifest window intersecting the selected history through the original window reader. Each exact history window must complete. Logs keep `battleLogRows` order; tables keep their actual sorting, raw row operands and numeric cells. Full models omit deferred country/location text and fleet details. Empty newest views keep one preparation identity as earlier rows mount; ordinary builds retain viewport-only demand and expose no preparation.

**Files:** [BattleLogView.tsx](BattleLogView.tsx) owns requests/filtering/browsing; `BattleLog.tsx` composes LogList/Table; `BattleSides.tsx` renders sides; [battleRows.ts](battleRows.ts) validates rows/windows and re-exports generated types; `battleOptions.ts` and `battles.css` own settings/style. Adjacent tests cover the shared contract.

Shared framework conventions: [widgets](../README.md).
