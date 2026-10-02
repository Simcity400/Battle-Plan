# Construction log

Every construction project of the chosen countries, as a log (spec 2.2d, mock-up style-5): newest first, each row with its date, the game's picture of the building from the construction queue, the country with its flag as recorded, the building and its levels, where it is, its status and the factories on it. Repairs carry the queue's own repair frame over the picture. With Live on it follows playback: a project appears when the queue read first lists it, and enters again at the top when a read shows it build a level or end. Live off shows the whole campaign as it stood at the branch's end; a date range shows the rows dated inside it.

It reads the `construction-log` product (`crates/dashboard/src/products/construction_log.rs`, one reply per branch with every country's projects). The product's notes give each field's meaning and the evidence for how a project starts, builds, finishes and is removed; in short:

- **Started** is the line's recorded creation hour (exact).
- **Levels built** are the rises of its target building's level (a repair: its healthy level), recorded every hour, that the line itself shows it made: its remaining levels fell, or its progress fell by about a level's cost (a completion the player's added level hid). Only a rise at the game's daily construction update (midnight) can be its: a decision or event choice at another hour never is. A level rise of a damaged building is never its either: the game repairs before it builds. Its levels are dated by their rises only when the possible completions equal the completions, counting a multi-level rise as its count; otherwise they are dated "by" the read. Extra possible completions of a listed line widen both built and total counts ("1–2 of 2–3"), combined with unaccounted levels and the earlier-work "+", only from the read that shows them. Sampled speed bounds nothing between reads, since factories can be reassigned at any hour. A project first listed at the same building may have repaired or built there from the hour it was created, so a rise from then on is neither's. A railway's segments are known only by the read that shows its remaining segments fall ("by"). When work before its first read was not recorded, both counts are lower bounds with a "+" ("1+ of 2+", or "0–1+ of 2+" with unaccounted levels), even for a single known level; the tooltip and screen-reader sentence say so. Levels its remaining count lost where the building cannot be checked (an hour not recorded, or another project at the same place, including one first listed at that read) are shown as not recorded: "0–1 of 2" on the row and in the tooltip, never dropped from the count, and still a range once it ends.
- **Finished** at the hour the last level rose (a repair: only once its building is fully healthy again, so damage in between is not mistaken for a finish; a line with new levels queued keeps repairing until its last new level). A railway finishes only at a midnight rise of its last province to the target level. Leaving the queue proves nothing built, since a cancelled line leaves too, so a line with new levels is finished only when its possible completions equal its levels left, and unknown when there are more. **Removed** when it left with fewer possible completions than its levels left, so some level was never built (cancelled, or its builder lost the place: the recording does not say which, and the row says "by" the read); **Left the queue** when whether it finished is not known (hours not recorded, two projects at one building, rises that cannot be told apart from its own, nothing left to build, or a railway whose last province never rose). A repair, then new line's tooltip dates the repair and the build separately ("Repaired", "Built").
- Nothing shows before the read that establishes it: a building that rose at midnight shows as built from the read three hours later.
- **Repair or new, per stretch:** a line repairs while it has levels to repair and builds its new levels otherwise, so one line can be a repair, then new ("Repairing", "Building, after repair", "Repaired and built"; the queue's repair frame shows while it repairs). The Projects setting counts such a line under both.
- **Levels added while under way** (the player queued more, or new damage) are counted when the line built more than its remaining count fell, or its building was damaged while the line was open, so "N of M" never loses a level; extra possible completions of a listed line may have been both built and added, so the added count also stays a range. Damage repaired between two reads counts even when neither read shows repair work, so such a line also shows under Repairs.
- The construction-IC capture being built (each line's daily IC) will make completions exact; until then a focus or effect at the very update the line could have finished, while the line was cancelled, reads as the line's.
- **Factories** are those assigned at the read in view (at its last read once it ended); "—" after a gap until the next read. Repairs often have none: the game repairs at a base rate without factories.
- Between a gap or failed queue read and the next successful read, an open project's status and pending tooltip dates say "Not recorded" in the same muted treatment as unread factories, with no repair marker.

Hover or reach a row with the arrow keys for its tooltip (started, finished, levels, factories, cost, and for railways the level and route), read out for the keyboard; right-click, Enter or the context-menu key offers Jump to its start, its latest change and its end.

## Settings

- **Countries**: all countries (default), chosen countries, or a smart group; a group counts a project by its country's membership when the project was first read.
- **Building types**: the types this campaign's projects name, named and pictured from content (never a fixed list); none chosen shows every type. The chips above the log set the same option.
- **Projects**: new buildings and repairs, only new (railways count as new), or only repairs.
- **Order**: latest change (default) or start date.
- The log settings every log shares: density, month headers, the new-row animation and the filter chips.

## Mod compatibility

- Building types, their names and their pictures come from the recording and the recorded content stack: the types are those the queue recorded (lexer ids named by the recorded dictionary), named by their own localisation key and pictured by their `icon_frame` in the strip the exe's queue entry uses (`production_building_line_entry` / `picture`, cited in the product). A mod's types appear in the chips and settings by themselves; nothing lists vanilla buildings.
- Whatever content cannot give falls back visibly: the script name (or "Building type N"), "State N", and a "?" placeholder for art. A building with no `icon_frame`, one that is not a positive whole number (an `@variable`, a scripted value), one repeated with different frames, or one past the strip is refused and named in a counted warning.
- Railways rest only on the exe's railway template token (0x4cc1, `CBuildingDatabase::initialize@140677d30`), named through the recorded dictionary.
- Tags are opaque: kept as arrays end to end, any characters, spaces included.
- State names come from content; renames made while the campaign runs are not recorded.

## Differences from the mock-up

- One row per project rather than one per event, since the spec asks for started and finished on the project; a project that changes re-enters at the top (with Order: latest change), so its latest event is where a feed would show it.
- The status is a coloured tag (Building, Built, Repairing, Repaired, Removed, Left the queue) and the right-hand value is the factories on it, where the spending log shows a cost.
- The building-type chips are the recording's own types, so a long list wraps; they can be turned off.

## Not yet

- States renamed while the campaign runs show their content name: state renames are not recorded.
- A project started and finished between two daily reads is never listed (the product warns).
- Whether a removal was a cancellation or a lost state is not recorded.
