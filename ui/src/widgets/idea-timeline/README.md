# Idea timeline

One idea category over time, as the game's political screen groups ideas: a lane per law or designer slot and per advisor seat, with bars for who held it and when. The left column names each slot's holder at the playback moment, cross-fading when it changes; empty slots are dashed. This is the approved timeline look (mock-up politics-4).

There is one widget kind for every category. Its **Category** setting lists the campaign's own categories from content (those on the political screen, not hidden, with slots), named as the game names them, so a mod's categories appear with no code change. A title that still names the previous category follows the new one.

## Data

- **Product `idea-history`** (`crates/dashboard/src/products/idea_history.rs`) for the widget's country: spans of every idea held, each idea's category and slot, the content's categories with their slots, seat counts and law order, and each idea's name and icon (advisors: the character's name and small portrait).
- `ActiveIdeas` is recorded once a day, so a bar starts on the day a holder was first seen and ends on the last day it was seen. Appointed advisors come from the recorded appointed-advisor list (`HiredAdvisors`, also daily), each with the seat and character the recording gives it, and the character's own name and portrait; seats fill in order.

## Settings

- **Category** (from content), **Laws** (escalation shading, icons or both) and **Advisors and designers** (portrait and name, name, or portrait).
- **Time:** Live, Whole campaign, Pinned or Date range. Zoom and Jump to work as in every time chart.

## Not yet

- Hovering a holder shows its slot and dates. The game's own idea tooltip (effects, targeted effects, traits, description) follows once built-in modifier labels are recorded (Wave 2 part A).
