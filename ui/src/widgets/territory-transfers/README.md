# Territory transfers

**What it shows:** Who gained and who lost states in peace settlements: a table of countries (states gained, lost, net, net victory points), or a table of every state that changed hands, from whom, to whom and when.

**Product(s) it reads:** The prepared peace-deals query gathers branch events and uses the existing product reply builder.  [`peace-deals`](../../../../crates/dashboard-peace-deals/src/products/peace_deals.rs) supplies each settlement's transfer graph, including states handed to a country released in the same settlement; [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) supplies dated state names; [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) resolves chosen groups.

**Settings:** Whole world by default; the header's country chip narrows it to transfers involving the chosen countries or groups. Show: Countries (default), Transfers, or a pinned country's transfers. Scope: whole campaign or one conference. Breakdown inline or beside the widget; shared table density, cell bars and a saved sort for each table. Campaign, Follow, Pin and Range.

State-owner comparisons and dated finalizer joins require earlier observations,
so nothing is sampled past the prepared prefix. Before preparation covers the
branch the tables hold the prefix's settlements, with the shared
"Prepared A to B; the rest is still being prepared." note above them; with
nothing prepared yet the widget shows "This range is still being prepared.".

**Notable behaviour:**

- Countries are native holders (slot and country ID): a renamed country stays one row under its newest name in view, and a reused tag stays separate. Each side of a transfer is named and flagged at its own read. A state transferred twice counts twice.
- Countries sorts by Net, winners first. Net and Net VP are signed in the positive or error colour. Net VP is unknown (—) when any of the country's states lacks victory points; no partial sum is shown as known.
- A row's tooltip lists the countries it gained from and lost to, with counts. Clicking a country opens its transfers at the shown moment (Follow and Pin as widget); there the country is dimmed and VP is signed from its side.
- Transfers sorts newest first. A state handed to a country released in the same settlement reads "· released". Rows show plain dates. The tooltip gives the hour, the war, how the settlement moved the state where it is recorded (the content's own action name, released, subject of), and its victory points. An outcome found by comparing two reads says "By" that hour, and when the state was still unchanged.
- Transfers appear as playback reaches both their date and the observation that supplied them; outside Campaign, a state's victory points wait for their own read ("Not read yet"). Range includes both ends; backward seeks remove later rows at once.
- Gaps in the hourly settlement, ownership and war reads within the shown time add one muted line, "Some transfers may be missing.", with the span as its hint. The conference roster's once-a-day reads are its cadence and never count. With nothing to show, the body says only "No territory transfers yet." (or "… for these countries yet.").
- Older replies without released states in the graphs still list them from the release item, without victory points, naming the former owner only when the settlement had one. Pins from the earlier flow diagram open as the Transfers table limited to their saved giver → taker links; links missing from the scope are named and counted. Version 1's size and ribbon-colour settings stay stored, unused.

**Files:** [TerritoryTransfersWidget.tsx](TerritoryTransfersWidget.tsx) renders the views and drill; [TransferTables.tsx](TransferTables.tsx) defines both tables' columns; [TransferDrill.tsx](TransferDrill.tsx) is a country's breakdown; [territoryData.ts](territoryData.ts) decodes occurrences, balances, gaps and how; [territoryTooltip.ts](territoryTooltip.ts) builds tooltips and label requests; [transferDrillData.ts](transferDrillData.ts) owns time and pinned selections; `useConferenceChoices.ts` lists conferences. Tests beside them use [transfers.fixture.ts](transfers.fixture.ts). CSS, `definition.ts` and `index.ts` own style, settings and registration.

Shared framework conventions: [widgets](../README.md).
