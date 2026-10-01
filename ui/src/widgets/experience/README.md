# Experience

Army, navy and air experience per country over time, in the chart kit's approved Lines look.

- **Banked:** the experience held at each daily read, the default measure. Each value holds from its read until the next; the line breaks at a recording gap, unavailable data or a different country taking the tag.
- **Change per day:** banked now minus banked at a read exactly one day earlier, for the same country in an unbroken recorded stretch. Without that read the change is **Not known**, never zero. It is a net change, including experience spent and the game's cap.
- **Branches:** army, navy and air can each be switched off; all start on. One country uses the first three line colours in fixed branch order, even when another branch is off. Comparisons use each country's recorded map colour, with the same optional Appearance adjustment as Political power; army is solid, navy has the kit's short dash and air its next dash. A key names the dashes.
- **Countries:** one by default, with comparison and the same smart groups as Political power. A group counted at each hour draws only while the country belongs; badges mark joins and leaves. There is no aggregate. A country without reads says **Not in this recording**.
- **End labels:** "Army", "Navy" and "Air" for one country; "Country · Branch" for comparisons, so lines of one country stay apart, with the exact formatted value. Labels stay at the last drawn hour when the playhead or view ends in a gap. The kit supplies the leader lines and ringed end dots.
- **Tooltip:** each shown branch's banked value or net change, rescaled once from the reply's raw strings with the kit's amount formatting. The date is the read's date, and a carried value names its read and says experience is read once a day. Country names and flags come from the hovered hour. In a short window the tooltip fits and keeps the full content available to screen readers.
- **Keyboard:** arrow keys step through every read; Home and End go to the ends and Escape leaves. A range starting between reads also reaches the carried value at its start. The tooltip is shown and read out; the context-menu key opens Jump to.
- **Right-click** jumps to that moment.
- In whole-campaign playback lines and hover stop at the playhead unless **Show after the playhead** is on. A date range draws, fits and hovers only its hours.
- **Settings:** measure and the three branches; playback mode, window and **Show after the playhead**; end labels; value axis from/to and log scale.

## Data

**Product `country-experience`** ([country_experience.rs](../../../../crates/dashboard/src/products/country_experience.rs)) supplies each country's `CountryExperience` reads: the first hour read, last hour held, exact banked raws for all three branches, and their net changes or the reason they are unknown. The reply carries the engine's scale and the comparison interval; the widget hard-codes neither. Tags are opaque lookup keys.

The product dates reads with `metadata::slot_read` and resolves the country with `tagged_country` and `link_slot`, as Fuel does. It checks identity and continuity every recorded hour and compares actual reads, including reads outside the regular daily slot when a matching earlier read exists. It never substitutes a carried value for the read required a day earlier. Synthetic replay tests cover carrying, gaps, tag-holder changes, unavailable tables and links, missing countries and reads, off-slot reads, and exact signed arithmetic.

The engine's daily and training accumulators are **not** the measure called Change per day. The daily update adds passive gain and resets them at the day boundary; a later daily-slot read contains only part of the day. The code's module notes keep this evidence beside the calculation.

**Built from the kit:** `TimeChart`, `drawLine`, `placeEndLabels` and `lineEndHour`; palette line styles and dashes; `clipToSpans`, `runAt`, `readAt` and `readNote`; `rescale`, amount formatting and `fitTooltip`; exact-hour country labels; shared country line colours/styles and line settings; groups, states and Jump to. Only experience's values, reply validation and tooltip wording live here.

## Not yet

- **Gross daily gain:** spending and the cap cannot be separated from the recorded banked change, and the partial-day accumulators cannot provide a full day's income. The widget therefore shows the net change only.
- **Attribution:** no breakdown of what earned or spent experience is built.
