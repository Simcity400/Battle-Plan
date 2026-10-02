# Production lines

One country's production screen (mock-up economy-1). The widget opens on the whole campaign (Live off), as history widgets do; the Live switch in its header shows the lines running at the playback moment.

- **Live** (following playback, or a pinned moment): one row per line running then, like the game's production tab.
  - Each row shows the equipment's art, the variant's name and its equipment type, as the game's row shows them. It also shows:
    - factories assigned of those requested, with a bar and a tick for requested, on one scale for the whole campaign;
    - efficiency, as the game rounds it, with a bar to 100%;
    - output per week;
    - the units the line has made so far.
  - **More columns** (setting) add the queue place, IC per unit and the military-industrial organisation (its icon and name).
  - **Order** (setting): the game's order (the queue place, default), factories assigned, or output. Output compares lines by IC a day, since units of different equipment do not compare.
  - Rows slide to their new place when the order changes, a new line flashes in and an ended line fades out (the kit's `useRowMotion`); hover, focus, right-click and the menu key work through the kit's `useRowTooltips`.
  - A month of the lines' reads is loaded at a time, with the next one during playback so it is ready at the month's edge.
  - The caption says when the values shown were read. Where no census of the country's lines is recorded (a gap, before the first readable census, an unavailable table) the table says the lines are not recorded; "No production lines running" is shown only where a complete census recorded none.
- **History** (Live off, or a date range): one lane per line across the campaign, in the order the lines were first seen.
  - The shared timeline draws only visible lanes and a viewport of overscan on either side into its canvas; scrolling and keyboard navigation still reach every lifetime at the same lane positions.
  - The band's thickness is the factories assigned at each read, on one scale for every lane. It is accented while the line runs and grey once it has ended.
  - A variant switch starts a new span of the same lane.
  - Beside the lane: the variant's art and name and the line's factories at the end shown, or "Ended by <day>" (the first read without it). A line whose recording just stopped (a change of country identity, or the end of the recording) says "Last recorded <day>".
  - An ended lane gives the line's final count (and what it made of each variant) from its end onwards, but never before the census that published that count: hovering the lane after its end, or any of its bars (which the keyboard steps through) once the view is past its end.
  - A line made and deleted between two daily reads, which no census lists, gets a lane over the hours between the two reads (its factories are not known, so a plain bar in the ended look), with its variant and final count from its output rows. One that ran before the first complete census is not given a creation date: its lane runs from when the observation began and says it ended by that census.
  - The name and art beside a lane are those of the variant it made at the end shown; the kit's timeline redraws them whenever they change, even under the same line.
  - Ended lines can be hidden (setting).
- **Hover** (or keyboard focus on a row, or arrow keys on the timeline) shows the line's tooltip: factories, efficiency, output, IC per unit, queue place, units still ordered, the organisation and the units made so far (per variant when the line switched: a variant made under several organisations is one row, exact only when each of its counts is), with what they count. Where a line is not recorded (a gap, or before it started) the tooltip says so, never another hour's factories. The tooltip is fitted to the window by the kit's `fitTooltip`: a line with more variant parts than fit shows its total and how many variants it counts, and a screen reader still hears every part.
  - On the timeline, the tooltip loads the month of reads behind the hour hovered; until it arrives it shows the line's variant and factories and says it is loading.
  - The keyboard stops at every daily read of a line (its efficiency and count change there even while its band does not), and a line made between two reads also stops where its final count was published.
- **Right-click** (or the context-menu key on a row) jumps to that moment: the read the row shows, or the hour under the pointer on the timeline.
- **Screen readers:** each row's description is its whole tooltip. The timeline describes every lane at the end shown.

## Data

Two products (`crates/dashboard/src/products/production_lines.rs`), so no reply grows with the campaign; each is measured before it is sent and refused past its bound. **`production-lines`** supplies, for one country over the branch:

- each line's steps: its variant and factories over its reads, a new step only where either changes (what the history draws);
- the final count published when it ended, and when it ended;
- each produced variant's names and icon, and each organisation's name and icon, with the pictures once each;
- the hours its lines, output and organisations are recorded.

**`production-line-detail`** supplies one window of hours (30 days): every line's recorded `ProductionLines` values as runs, one per read and carried until the next, and its recorded output (`ProductionLineOutput`) at each read. A read that holds into the window from before it is sent with it.

The product module cites the evidence for each field. In short:

- IC per unit is `cost` (the cost after the organisation's bonus).
- Output per week is the game's weekly formula, `speed × 700000 / cost` fixed5 truncated, from the product (`equipment::weekly_units_raw`). The game's row shows this unit only when `speed < cost ≤ 7 × speed` (per day above, per month or year below); each is the same rate truncated in its own unit, and the widget shows every line per week so they compare.
- Efficiency is the recorded Q15 value (100% is 3276800); the row shows `(raw + 0x4000) >> 15` percent.
- A line is its native handle. A variant switch edits the same line, so the line keeps its total.

**Totals** are the collector's count of every unit each line delivered (`ProductionLineOutput`, one row per variant and organisation the line produced under; the total is their sum). The game keeps no count per line: `produced` is progress toward the next unit.

- A row the collector could not count exactly makes the total "Not exact", never a number.
- Output is carried between reads only while its table stays readable: from the first hour it is unavailable, the total says "Not recorded".
- A line already running when the branch's observation began counts "since <that day>"; one created since counts its whole life.
- Where the output table is unavailable (it stays so for the rest of a branch once a tick went unobserved), the total says "Not recorded".

**Names** follow the game's `CEquipmentVariant::GetDisplayName` (`equipment::variant_names`, shared with Equipment stockpile):

- the variant's own name when it has one;
- else its equipment type's key localised for its origin or creator country (`<TAG>_<key>`, else `<key>`);
- plus its suffix when the variant asks for it.

The type line is the key localised for the country in slot 0, as the production row asks. A variant is named only from its first definition read onwards (the kit's `entryAt`), and only where the shared `VariantCatalogue` knows them (read, and carried until the next read while the recording is unbroken and the table stays complete); elsewhere its name and icon are not recorded (an entry with `known` false), never an earlier or later read's, and the line shows "Variant <id>". A variant a line made only between two censuses (named only by its output) is named and pictured too.

**Equipment art** follows the production row's rule (`BindEquipmentVariantIcon`, spelt out in the replay README), through the recorded content stack (`equipment_icon.rs`), at each read with that hour's countries:

1. the variant's own icon entry, when active and it exists;
2. an auto-designed variant's (`"."`) graphic pool for its creator, with its ruling ideology group and graphical culture;
3. sprites named after the type and its archetype for the origin or creator's tag, then after the one technology enabling it (by id, then its `parent`) with that country's original tag (its own unless it is a dynamic country), its 2D culture or no prefix, then the type's family, then the graphic pool for that country;
4. else no icon.

The graphic pools are the same reader the map's 3D models use (`installed_content::equipment_graphics`). The country's tag comes from Countries and its original tag from CountryAppearance, and its tag-prefixed sprites are tried before its 2D culture is needed; the technology that enables a type is found under the lexer's case fold, as the game finds it (a technology enabling equipment through a script is refused alone, in a counted warning: a type it mentions is unknown only where the rule reaches the technology step); sprite names are matched exactly, as the game's sprite registry compares them; texture paths are found exactly as the texture loader reads them (an archive member only in its own case) (`ResolvedLayers::has_file`, which sees a mod's textures outside the coverage roots). Where the rule needs something the recording lacks at that hour (a ruling party, politics or appearance table not read, a 2D culture it would try, a pool `limit` other than a DLC test), the icon is unknown: the row shows a neutral mark whose hover says why, never a guess.

**Organisations** are known only where their table was read (and carried until the next read); elsewhere the column says "Not recorded", never an earlier read's name. They are named as the game names them: the name-key override, else the script key, localised preferring `<owner tag>_<key>`. Their icon is the stored sprite, else `GFX_<owner tag>_<key>`, `GFX_<key>`, then the game's `GFX_idea_unknown`; where none of them is declared (a mod that removed it), the column shows a neutral mark. A positive owner reference missing from CountryLinks leaves the name and fallback icon unknown; the column says "Unknown owner"; its hover, the History tooltip and the keyboard read-out explain why. An existing stored sprite remains known without the owner. A name that needs runtime scope shows the script key.

Everything comes from the recorded content, so a mod's equipment and organisations appear as the mod names and draws them. Without the game folders only a variant's own name is known, others show their script key, and there is no art.

**Freshness:** lines, their output and the organisations are read once a day and carried in between. A gap, an incomplete table or a change of country identity ends them, and carried rows are never shown for a country that was not read.

**Unreadable output reads:** a line made and ended between two daily reads publishes its only rows in the next output read. Where a due output read cannot be read (an unresolved country identity, or incomplete country tables, census or output), those lines may be missing, so it is never dropped silently. The product counts such reads in a warning and marks their hours (from the read before), and every History tooltip over them says lines made and ended there may be missing. A tag no country holds is not counted where the countries were read. A block of the recording, or the frames of one, that cannot be read is marked and counted the same way. A line whose final count lay in such a read shows it as not recorded, never its last count as a total. A line or output row whose producer the recording does not link to a country is counted in a warning; one naming a line of this country stays that line's, with its count not exact.

## Not recorded

- **Units made before the branch's observation began**, and while the recording has gaps.
- **The factories of lines made and deleted between two daily reads**: no census saw them, so only their variant and output are known.
