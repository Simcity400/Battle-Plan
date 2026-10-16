# Ships sunk log

**What it shows:** Recorded named ship sinkings, newest first, with victims, attribution, class, sea zone and available native battle titles.

**Product(s) it reads:** [`ships-sunk`](../../../../crates/dashboard/src/products/ships_sunk.rs) uses the shared [`sinkings`](../../../../crates/dashboard/src/products/sinkings.rs) retained-history scan; [`ships-sunk-battle-titles`](../../../../crates/dashboard/src/products/ships_sunk/battle_titles.rs) resolves drawn titles through [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs).

The manifest supplies class choices; normal log demand holds playback and visible replay windows. Earlier/Latest sinkings reaches every native-date window. Late retained observations repair their native window, and its stable scan indices continue to identify title requests.
Published rows and attribution patches use shared dated events; continuation
retains unresolved operands and class facts. Drawn battle titles read their
immutable supplying-frame projection from prepared events.

**Settings:** All countries by default; victim/sinker/both, recorded class chips and shared log controls; Campaign, Follow, Pin and Range. Empty class selection means every class.

**Notable behaviour:**

- Sink hour dates order, ranges and Jump; `known_at` gates visibility. Entries never appear before their first supplying read. Country facts use event history when possible, otherwise that first read with **as of** wording; victim/sinker dates can differ.
- Identical native records remain legitimate multiset occurrences. Retention pruning can hide never-observed events; empty logs do not claim a complete lifetime log. Empty filtered rows say no sinkings recorded, not no sinkings occurred.
- Hostile, assisted and non-hostile causes keep distinct wording. Stored ship/cause text requires the recording language; class names/art resolve through content; unknown classes use a readable label and missing artwork uses an anchor fallback. No cause or missing name is invented.
- Class labels preserve localized text, including empty titles, and recorded literal class names. An unresolved class shows **Class name unavailable**; keys belong in chip hints and log details. Classes with the same title keep separate filter values without adding their keys to the title.
- Battle references are full live-combat handles, not retained-result keys. Titles use the native content formatter and supplying-read geography; missing operands leave the sinking visible and report counted diagnostics without a missing-title row. No later name reconstructs an unseen event-time title.
- Manual tags follow native holders and aliases at each endpoint's naming hour; identity gaps do not match. Drawn rows alone request looks/titles; stale source rows cannot reuse them. Shared LogList owns motion, keyboard and menus.
- Parity builds expose the complete ordered filtered view model: every sinking operand, class/title reference, recorded name and place, with the history's recorded boundaries. Battle titles and dated country labels continue loading only for drawn rows. Visible text and screenshots cover mounted descriptions without an offscreen title/name sweep. Ordinary builds expose no preparation or row model.

**Files:** [ShipsSunkWidget.tsx](ShipsSunkWidget.tsx) renders; `shipsSunkData.ts` shapes rows/filters/tooltips and `useShipsSunk.ts` loads the prepared manifest/titles; `shipsSunkWindows.ts` combines reached windows with their own dictionaries; adjacent tests cover these rules. `definition.ts` and `index.ts` declare settings and registration. [Naval losses](../naval-losses/README.md) shares the scan.

Shared framework conventions: [widgets](../README.md).
