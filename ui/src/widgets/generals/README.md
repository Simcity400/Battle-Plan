# Generals leaderboard

**What it shows:** Recorded generals and admirals with portraits, ranks, skills, attributes, traits and decided/unresolved battle counts. Activate a row for its career.

**Product(s) it reads:** [`unit-leaders`](../../../../crates/dashboard/src/products/unit_leaders.rs) supplies histories/choices; [`unit-leader-portraits`](../../../../crates/dashboard/src/products/unit_leaders/portraits.rs) supplies mounted-row artwork.

**Settings:** All countries/leaders by default; manual countries and smart groups, land/navy kind, skill/won/lost/unresolved/win-rate sort, table bars/ranks/density, inline/pop-out career and Grow with playback; Campaign, Follow or Pin.

**Notable behaviour:**

- Full references include type. Country selection admits leaders held by selected native countries and preserves their careers across transfers; group filtering keeps its counted dates. Native visibility hides captured/hidden/undecidable leaders with named refusals.
- Content defines rank, skill ceiling, attributes, traits and art. Trait experience is learning progress, never a holding. Defined empty names stay empty; only missing names fall back. Missing owners and attributes remain explicit. Missing portraits use a helmet icon; missing trait icons show name chips.
- Fought counts each observed combat once per leader, since the first recorded leader hour. Known wins/losses use exact native result links; disappearance implies no result. Win rate uses decided battles only. Fact dates and first supplying reads stay separate, preventing backfill. The leaderboard receives only the hours each participation, win and loss becomes visible; careers keep each battle's dates and reads, and both count the same at every hour.
- Gaps and missed battle coverage retain counts with an incomplete-count status. Career holdings after unknown reads keep bounded dates. Portraits load only for mounted rows, separately from histories.
- Career drills from Campaign/Pin hold the row hour. Follow uses the actual clock plus alignment; Pin keeps the day or Follow. Changing client/source revision/branch closes the drill and revokes stale callbacks.

**Files:** [GeneralsWidget.tsx](GeneralsWidget.tsx) renders table/drill; `generalsData.ts` and `useLeaders.ts` own validation/loading; `LeaderIdentity.tsx`, `Attributes.tsx`, `portraits.ts` and `skillDisplay.ts` shape cells/art. Adjacent tests cover these rules. `generals.css`, `definition.ts` and `index.ts` own styling, settings and registration; [CareerView](../general-career/CareerView.tsx) renders detail.

Shared framework conventions: [widgets](../README.md).
