# World tension & threat

World tension over time, stacked by the countries whose actions generated it (mock-up diplo-1). A world widget (`countries.all`): it shows every country by default; chosen countries or a group narrow the layers to them (ranked among them), and every other country is "Other".

- **Layers:** a stacked area per country, in its recorded map colour (adjusted apart when the app's country colours are fixed; grey when none is recorded), and "Other" for every other country. The layers are the countries contributing most at the moment shown (the playhead, else the end of the hours shown), largest at the bottom. **Layers (setting):** Top 5 + Other (default) or Top 8 + Other. For a group counted at each hour, a named layer shows its country only during its recorded membership spans; its contribution goes to Other before it joins and after it leaves, including between daily reads. Groups counted at the end or anyone ever keep the chosen countries' whole histories. The stack is rebuilt only when the ranking, membership or value mode changes.
- **What the layers show (setting):** by default each country's **share of world tension**, so the layers add up to world tension, as signed off. **Recorded contributions** stacks each contribution exactly as recorded instead; their sum can pass the game's 100% cap (133.5% in the W4 part-M recording, with world tension at 100%), and a peace treaty's contribution is negative. Negative parts stack below zero and positive ones above it (the kit's signed stack).
- **World tension** is a line on top of the layers, joining exactly the same sampled hours with the same straight segments as the band edges, including when zoomed or cut by the playhead. The total keeps its own recorded value when contributions differ from it. The value axis is 0–100% and grows to whatever is drawn beyond it, so nothing is ever cut off. Its labels take the decimals the ticks drawn need (the kit's exact `formatAxisValue`), so ticks zoomed in close (12.31%, 12.32%) never read the same.
- **Values beside the chart (setting, on):** world tension and each layer at the moment shown, with its flag and name as recorded near then (`useCountryLooks`, read about once a month of playback at a recorded hour of the moment's own stretch: each part, name and colour or flag, is held only while its next read loads and never across a gap, and one the recording does not give is not known; a flag that cannot be read, as without the game folders, leaves the recorded colour and name in place); the rows glide to their new order when the ranking changes (`useRowMotion`). "Other" counts the countries in it. In share mode a note gives what the sources add up to.
- **Event markers (setting, on):** a small triangle along the top in the causing country's colour for each event dated inside the hours shown. Hovering one shows the event (the game's own text for it), the country, the date and the tension it generated; when the game changed that value later, what it adds by the moment shown too. Several events of one hour are listed together; when they do not fit the window, a compact form gives their count and total ("20 events · +12.5% tension") and the largest ones, and the full list is read out.
- **Hover:** a crosshair and every country contributing that day, each named and flagged as recorded at the hour hovered (`useHoverLabels`, asking only for the tooltip's countries), with its recorded contribution, largest first (only members at the hovered hour for a group counted at each hour), then contributions from countries the recording never names, and any event of that hour. When the sources add up to more (or less) than world tension, the tooltip says so. A value read earlier than the hour hovered says when it was read. The tooltip is fitted to the window (`fitTooltip`).
- **Keyboard:** with the chart focused, the arrow keys step through every read and every event marked, Home and End go to either end, and the tooltip is read out. A stop at an event dated where no read holds the hour (inside a gap) shows the event as hovering its marker does, not only "Not recorded". The context-menu key opens Jump to.
- **Right-click** jumps to that moment.
- **Gaps:** an hour without a read is not recorded: the layers and the line break there, and its tooltip says "Not recorded". Values are never carried across a gap.
- **Settings:** the playback mode, window and **Show after the playhead**; layers, what they show, event markers and the values beside the chart.

## Data

**Product `world-tension`** (`crates/dashboard/src/products/world_tension.rs`) supplies, for a whole branch:

- each read of world tension (`WorldTension.total_raw`), the hour it was read and the last hour it holds, and the sum of every country's contribution before the game's cap;
- each country's contribution (`ThreatCountryAggregates`) as runs of reads where it was not zero; in a read without a run it contributed a recorded zero. A country is its canonical slot, named by the tag it held (`slot` and `tag`): two slots contributing under one tag (a country whose sources still decay after its tag passed to another) are two countries and two layers, never one band. A run starts a segment only after hours the recording lacks; the stack also breaks wherever a layer's own run starts one;
- each event (`ThreatSources`): the country that caused it, the hour the game created it, its reason as the game wrote it, and what it adds (`final_raw`) from its first read on.


**An event's identity** is its source's own recorded country reference, second country, date and reason (and its place among sources equal in all four), never the country slot the reference resolves to: a source read while its country is not yet named and named at a later read stays one event, which takes the country then.

Chosen countries and hourly group membership compare `worldFilter`'s country
identities through the shared `Lineage`. A slot is tied to its first recorded
metadata observation in the branch, else its earliest contribution observation;
its country then follows recorded renames. Raw layers still keep their separate
slot-and-tag keys. Names and flags resolve that country's tag at the hour shown
or hovered, and `countryMembership` clips membership across its recorded tags.

**Event names** are the game's text: the source's recorded `reason` ("Declared war on Poland", "Peace - Treaty of Wien"), in the recording's language, with the engine's colour codes removed (`catalogue::display_text`). Nothing is named by the widget. Invalid UTF-8 or controls plain text cannot show are refused with their reason. An event whose country the recording never names says so.

**A country that no longer exists** keeps its sources until they decay, so its contribution keeps the tag its country slot was last recorded with, and its events their country. When another slot holds that tag meanwhile, the two are separate layers, each in the tag's recorded colour and name. A slot's tag and a link's slot are remembered from every hour recording `Countries` and `CountryLinks`, whether or not the tension tables were recorded then.

**Share of world tension** is each contribution times world tension over the sum of every contribution, computed exactly from the recorded integers (`shareRaw`); when the sum is not positive world tension is zero and so is every share.

**Built from the kit:** the time chart with its triangle event markers (`badges.ts`), the signed stack (`charts/stack.ts`: `buildStack` from a run per read and layer, `stackedSegments`, `stackExtent`, drawn with `drawBands`, and `drawBandLine` for the total on the same samples), `clipToSpans` and `groupRuns`/`groupMembers` for hourly membership, `fitTooltip` (with its `full` option for grouped events), keyboard stops over every read, `formatAxisValue` for the value axis, `useRowMotion` for the list, `nudgeApart` for fixed colours, `worldFilter` for chosen countries, and the country-facts hooks (`useCountryLooks` for the layers' colours and the list, `useHoverLabels` for tooltips). Only what the values mean (ranking, Other, shares and the rows) lives in this folder.

## Not yet

- **The second country of a source** (`ThreatSources.second_country`) is recorded, but its role in the game is not established, so it is not shown. In part M it is empty for every source.
- **Events dated before the recording** (131 of part M's 132) are never marked, as their date is outside the branch; their contributions still show in the layers.
- **Decay** of a source's contribution day by day is shown only through the layers; the markers give what an event generated, not how much of it is left.
