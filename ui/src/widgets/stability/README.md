# Stability & War support

Stability and war support over time in one widget, as signed off (mock-ups politics-1/2): each country in its map colour, **stability solid and war support dashed** (a key under the legend names the line styles), on a **fixed 0–100% scale** by default with a **Zoom** option (both bounds fitted to the values drawn in view, following the view as it moves), draw-in playback by default (in whole-campaign playback the lines stop at the playhead unless **Show after the playhead** is on) and values at the line ends ("Stability 96.0%"). Either line can be switched off. Right-click jumps to that moment.

## Data

- **Product `country-stability`** (`crates/dashboard/src/products/country_stability.rs`): the displayed values, computed from each country's recorded inputs with the engine's arithmetic (`GetStability`, `GetWarSupport`), as exact runs, and `carried`, the hours whose values come from an earlier read.
- **The breakdown** is the same product with an `hour`: the read behind the hovered hour (`readHourAt`) is fetched once and kept. It lists the formula's terms and, for the modifier term, the game's own tooltip lines in tree order. Each line is named in the viewer's language from content (an idea, advisor or leader, ideology, static or dynamic modifier) and prefixed with what it is ("National spirit", its law or advisor slot, "Country leader"). A line the recorder could not pair keeps the game's own text in the recording's language, in grey, with a note. Whatever the lines do not add up to is its own "Other / not identified" row; nothing is invented.
- **Products `country-styles` and `country-groups`**, as for Political power.

## Hover

One country: the date, then **Stability** and **War support** each with its signed parts and a bold **Total**, one decimal, green or red by sign (mock-up politics-1). Values carried from an earlier day say which day they were read; the breakdown, character names included, is as recorded at that read. Several countries: one line per country with its two values (a † marks values carried from an earlier read, dated in one note); choosing one country shows the breakdown. A breakdown that fails to load is asked for again a few times, with growing delays, and whenever its day is hovered anew, including after the pointer leaves the chart and comes back. With the keyboard, while focus is in the widget, the breakdown of the moment shown loads once playback rests and every line is announced, with the read it comes from; the current values say when they were carried from an earlier read. Values at the line ends are each line's last sample drawn, never past the recorded hours, and only as many labels as fit are drawn. A country without a recorded map colour stays grey, even with adjusted colours.

## Not yet

- **Event markers** (wars and capitulations, faction changes, government changes, focuses completed, each toggleable) follow in the next change.
- **Drill-down** (mock-up drill-1): the parts at a day as bars and over time as a stacked chart, with Follow playback and Pin as widget, follows after the markers.
