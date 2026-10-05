# Agency comparison

The `agency-comparison` product supplies every country's recorded agency screen
values. The shared style-5 table owns sort controls, virtual rows, in-cell bars,
rank-change markers, row motion, tooltips and Jump to. Flags and country names
are requested at the displayed hour; sorting by country loads names for all
candidate rows while flag requests remain limited to mounted rows.
Chosen-country requests are bounded to those countries; a group requests its
recorded membership union for local filtering, while All and Top N request all
candidates. Pending or empty choices never cause an all-country request. Equal
values follow the native UTF-8 tag order in both the backend and shared table.

Created agencies are shown by default. **Include agencies not created** shows
their recorded scalars and the game's content-localised default agency name;
those rows are muted. Created names preserve the stored literal, including an
empty name. The defence header uses the content-localised label (Defence when
content is unavailable), and cells show only the native-formatted number. The
full game text remains in the tooltip. Both come from the backend; the
column sorts on the exact raw value. Active / maximum uses the two values shown
on the native agency screen, with a utilisation bar, never the recruitment
admission ceiling. Daily operative-list counts carry a separate read date in
the tooltip. Missing values stay missing; unavailable hours show a gap state.
Equal agency-screen projections compress into one run even while undisplayed
creation/upgrade work and the recruitment admission ceiling change. Separately
dated roster counts use their own runs and shared read-hour list. An equal
daily re-read updates the tooltip's supplying date without splitting the
agency-screen run; unavailable roster hours remain unknown.

Country and agency columns share the width left after content-sized numeric
columns and truncate with full names in their titles. Narrow widths keep the
Agency column and truncate it; the shared kit has no column priorities.

Agency upgrades read one adapter result. Until acquired upgrade levels are
captured, the column says **Not recorded** and its tooltip explains why elapsed
upgrade work is a different measurement. Later capture can fill the adapter
with content names/icons without changing this widget's data path.
Its unavailable reason is carried once per reply; future upgrade identities
include both the recorded branch and upgrade tokens.

Choices where the specification is silent: defence descending by default;
active / maximum sorts on active count; Top N ranks by a separate setting,
default defence (active or maximum are available); all created agencies by
default, with selected countries and recorded groups narrowing them. The
shared table's comfortable density, cell bars and rank-change markers default
on. Operative bars use the country's recorded positive maximum; defence bars
fit the filtered table's largest raw value. Right-click jumps to the read hour.
The widget starts in Campaign mode, with Live and a pinned moment available;
its minimum is 4×6 grid units and default is 10×12. Country names use the shared
table's locale-aware numeric collation, with native UTF-8 tag ties.

Synthetic Node tests cover 5,000 countries, exact ranking, hourly gaps and
missing rows, independently dated rosters, localised never-created names and
the unavailable upgrades adapter. Browser motion/layout and live data remain
separate checks.
