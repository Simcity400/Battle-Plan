# Research history

## What it shows

The order each research slot worked through, as **Columns** (the default) or a **List**. The catalogue also offers it as **Research list**, the same widget opening on List. What each slot researches now is the separate [Active research](../active-research/README.md) widget.

- **Columns**: one even column per slot, a dense list of that slot's completions in order, oldest at the top. Each row has the technology's picture, its name on one line and its completion date under it. Columns are not kept level; the only offset is a slot that opened during the recording, which starts at the row the other slots had reached by then under an "Opened by" marker. Live and Pin add a Now row under the columns: each slot's current technology with its latest read progress, or Idle with its saved days.
- **List**: one table row per completed technology: Date, Technology, Folder, Slot and Took. It sorts by any column, oldest first by default.

## Products it reads

[`research`](../../../../crates/dashboard-base/src/products/research.rs) supplies the reference reply. The prepared research query uses its shared observer and naming to supply the complete selected-country slot states, assignments, progress, modifier credits, localized text and research folders; playback resolves them locally. Pending coverage follows the shared preparation query path. [`technology-artwork`](../../../../crates/dashboard/src/products/technology_artwork.rs) loads pictures in small batches, so names and dates do not wait for them.

Assignment dates, completions and carried slot progress need earlier
observations; no keyframe preview replaces them. Before preparation covers the
branch, the columns and list show the prepared prefix with the shared
"Prepared A to B; the rest is still being prepared." status beneath; with
nothing prepared yet the status says "This range is still being prepared.".

## Settings

Country and campaign/follow/pin; Whole campaign is the default. View (Columns or List), switched from the header too; Research folders, a filter also in the widget; and, for the List, Sort by and Density.

## Notable behaviour

- Dates are shown plainly: a completion's date is the read at which it had left the slot, and its tooltip says "Completed by …" with the last read still researching it. Started is bounded the same way, and Took counts from its first assignment, including earlier slots, as its time breakdown does. When the breakdown cannot confirm that assignment as the start (a start before the first read, or research resumed with earlier progress), Started reads "by" that assignment and Took is a dash.
- A completion classified at a later read appears only from that read, dated to when it left the slot. Completion belongs to the slot that finished it; a technology moved between slots says which slot it started in.
- Coverage, slots and slot states hold over the hours carried from a read until the next is due; progress keeps its read hour and is never interpolated. An hour without a read keeps the history known by then; the Now row reads "Unavailable".
- A stretch the recording lacks shows as a hatched "Not recorded" marker inside each column whose slot was open on both sides of it, and as a row in the List while sorted by date. A slot first listed after such a gap is "Recorded from" that date; a slot that stops being listed is "Closed by".
- Folder colours come from the shared categorical palette, given to the folders this country's whole history shows in the content's folder order, so they hold through playback. The filter dims other folders in Columns, keeping their shape, and removes them from the List.
- A completed cell or row opens its time breakdown (expected against took, with saved-day juggling, prior work, out-of-slot time, minimum speed and unexplained residuals kept separate). Right-click or Shift+F10 offers Jump to its date; the current technology's Jump goes to its earliest joined assignment.
- Older recordings that do not identify which occurrence a slot held show what each slot held, in order, dated "until" the read at which it had left; nothing claims a completion, and a note says so. Their progress is the latest read of that technology.
- Missing pictures use the flask. Image requests follow the shown cells, current technologies first, and repeated technologies share one image; failed batches report a counted diagnostic.

## Files

| File | Responsibility |
|---|---|
| [ResearchWidget.tsx](ResearchWidget.tsx) | Header switch, folder filter, Columns, List and the breakdown popover. |
| [researchLayout.ts](researchLayout.ts) | Lanes, opening offsets, gap markers, List rows, dates and tooltips; adjacent tests. |
| [researchData.ts](researchData.ts), [useResearch.ts](useResearch.ts) | Validation, playback lookups, held runs, latest progress, folder looks and the shared reply. |
| [researchJourney.ts](researchJourney.ts), [researchBreakdown.ts](researchBreakdown.ts) | Joined assignments and exact time/modifier parts; adjacent tests. |
| [ResearchExplanation.tsx](ResearchExplanation.tsx), [researchTimeBar.ts](researchTimeBar.ts) | The time breakdown and its segments. |
| [definition.ts](definition.ts), [index.ts](index.ts), [research.css](research.css) | Settings, the Research list card, registration and layout. |

Lane indexes are built once per immutable history reply. Playback selects revealed row prefixes by binary search, retaining unchanged rows; assignments, held runs and latest progress use indexed lookups. Columns use named native ordered lists on their shared scroll surface. Fitting histories keep native flow and grid stretching; overflowing histories mount the measured viewport rows. Authored padding and opening offsets remain part of row geometry and keyboard scrolling. Tab and Shift+Tab traverse every interactive history row across adjacent slots; opening and closing markers keep their places without becoming keyboard stops. Full histories are clipped when the time breakdown opens, keeping earlier work and hiding future observations.

Parity builds expose complete Columns rows, markers, offsets and exact current slot reads/progress without starting artwork or explanation work. List uses the shared table's complete model. Ordinary builds expose no model provider.
