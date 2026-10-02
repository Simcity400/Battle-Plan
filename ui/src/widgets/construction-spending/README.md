# Construction spending

Where one country's civilian-factory output went, as a sunburst (spec 2.4, mock-up style-3): **new buildings or repairs → building type → state**. The inner ring is the level zoomed into and the outer ring breaks it down. Click a slice to zoom in and the centre to go back; the side legend and the breadcrumb zoom too. With Live on it follows playback and grows read by read as the queue's progress builds up; Live off shows the whole campaign as it stood at the branch's end, a pinned moment shows it up to then, and a date range shows only what the reads inside it establish. Everything the sunburst does is the kit's ([charts/sunburst](../../charts/sunburst/)); this folder only turns the product into items.

It reads the `construction-spending` product (`crates/dashboard/src/products/construction_spending.rs`), which shares the Construction log's branch scan. The product's notes give the evidence; in short:

- **The measure** is the IC the queue turned into progress: each line's `produced` increments plus `cost` for every level it finished between two of its reads (a railway: its route's `produced`), never the stretch after a line's last read: the update that finished it recomputed its cost first, so no read shows what the finish took (it is listed as not counted). The recorded `speed` is not used: in part M it did not say what the day's update applied.
- **Dating**: an amount or an inferred repair interval shows from the read that establishes it, like the log's levels. A range counts the earlier hours an inferred repair covered once its end reaches that read.
- **Levels** count only where the log confirms them from the building's hourly record: every new level whose rise the line can claim (levels queued meanwhile included), none that was dropped from the line; where the record cannot settle the rises (hours not recorded, another line at the building, levels the log leaves unaccounted), the stretch is unknown.
- **Per stretch**: a line repairs while it has levels to repair and builds its new levels otherwise, so one line can repair, then build. Its repair reads join the Repairs ring (not measured), as does a stretch in which it repaired between two reads that showed no repair work, and its build reads the New buildings ring; a stretch that touched repair work is not counted for new levels.
- **Not counted, never zero**: stretches the reads do not establish (hours not recorded, a queue read that failed, a rise of the building the line does not show it made, levels lost unaccounted, a cost that changed while a level finished, two lines at one building, repair work on a new line, a line that left unfinished or whose ending is unknown) are counted by reason. The legend marks a slice with such parts "+?", the centre says how many, and the tooltip lists them by reason.
- **Repairs are not measured.** The game keeps a repair's progress on the building (a share of a level, part of it free) and sets the line's `produced` to zero; the recording does not show the factory output each day's repair took. Repairs are listed as "Not measured" with the reason, with their project counts, and never drawn as a slice.
- Projects already in the queue at its first read in the branch count only from that read; the note under the legend says how many.
- A date range counts the projects doing each kind of work during it (one interval per stretch of repairing or building, so a project that changes kind twice is counted only in its stretches, and a break in a project's reads ends its interval), and shows any category the reads in it gave an amount or an uncounted stretch; a range across a break says its hours were not recorded. With the new-or-repair level off, a project that repaired and built counts once.

Building types are named and pictured from the recorded content stack (the construction queue's own picture), so a mod's types appear with its names and art; a type without a name shows its script name, one without art a "?" placeholder in the legend; states show their content name, else "State N"; a railway the states of its route's ends ("A to B"). No building, state or tag is named in code, slices take the palette by position, and the country is the recorded tag as an opaque string.

## Settings

- **New or repair**, **Building type**, **State**: every level can be switched off. With the first off, new and repair parts of a type join (the type says its repair part is not measured).
- The sunburst settings every sunburst offers (`SUNBURST_OPTIONS`): **Growth** (Share, the default, keeps the circle full; Fill sweeps round toward the whole amount of what is shown), **Slice labels**, **Side legend**, **Breadcrumb** and **Hover dimming**, all on by default.

## Differences from the mock-up

- The legend keeps the slices in one order through playback (by their whole amount), rather than re-sorting as they grow, so rows never jump.
- Repairs have no slice: the mock-up's Repair ring would need an amount the recording does not have.
- More than eight slices under one parent fold their smallest (by whole amount, so the same ones throughout playback) into one "Other". Zooming into it shows its parts, folding their surplus for that view: the palette has eight colours and never cycles.
- The breadcrumb is on by default, as the spec's extras are; the mock-up's demo had it off.

## Not yet

- A project started and finished between two daily reads is never listed (the product warns). An empty sampled queue says "No construction recorded", with the daily-read limitation; it does not prove no construction happened.
- States renamed while the campaign runs show their content name: state renames are not recorded.
- Factory output lost when a line finishes with progress to spare is not counted, as `produced` never holds it.
