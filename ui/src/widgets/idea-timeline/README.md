# Idea timeline

One idea category over time, as the game's political screen groups ideas: a lane per law or designer slot and per advisor seat, with bars for who held it and when. The left column names each slot's holder at the playback moment, cross-fading when it changes; empty slots are dashed. This is the approved timeline look (mock-up politics-4).

There is one widget kind for every category. Its **Category** setting lists the campaign's own categories from content (those on the political screen, not hidden, with slots), named as the game names them, so a mod's categories appear with no code change. A title that still names the previous category follows the new one.

## Data

- **Product `idea-history`** (`crates/dashboard/src/products/idea_history.rs`) for the widget's country: spans of every idea held, each idea's category and slot, the content's categories with their slots, seat counts and law order, and each idea's name and icon (advisors: the character's name and small portrait).
- `ActiveIdeas` is recorded every hour, so a bar starts at the hour a holder was first seen and ends at the last hour it was seen. Laws, designers and spirits are in it.
- **Hired advisors are not.** They come from the recorded appointed-advisor list (`HiredAdvisors`, also daily), each with the seat and character the recording gives it and that character's own name and portrait; a holder whose character details were not recorded reads "Holder not recorded" rather than borrowing another holder's. Seats fill in order.

## Settings

- **Category** (from content), **Laws** (escalation shading, icons or both) and **Advisors and designers** (portrait and name, name, or portrait). With portraits only, a holder whose content has no portrait (a mod's missing art) keeps its name, so no bar is blank.
- **Time:** Live, Whole campaign, Pinned or Date range. Zoom and Jump to work as in every time chart.

## Not yet

- Hovering a holder shows the game's own idea tooltip (`dashboard/ideaTooltip.ts`): its icon, name, slot (and a law's level), the days it was observed, then its traits and effects formatted as the game formats them, and its description. For keyboard and screen-reader users the timeline describes each slot's holder at the moment shown, with its effects; Up and Down choose a lane and Left and Right step through its holders, showing and reading out each one's tooltip (Escape closes it).
