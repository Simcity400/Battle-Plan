# Special projects

## What it shows

Current lists a country's projects at the shown hour: projects in progress with their scientist, progress and iteration count, then completed and cancelled projects. History draws each project's progress steps over the campaign.

## Products it reads

[`special-projects`](../../../../crates/dashboard-special-projects/src/products/special_projects.rs) reads prepared project observations, completion history, content labels and recorded portraits. Preparation observes all countries once and evaluates scoped names in that same pass. The selected-country reply keeps its complete Current and History arrays; native selection roots filter observations before the shared episode builder assigns IDs. Numeric progress uses shared runs, dated changes use events, and supplying-read hours come from one branch marker intersected with country availability. The parity model is **Special project values**.

## Settings

Countries/groups, campaign/follow/pin/range. The Current/History switch sits in the header beside Live and is also the View setting; Grow with playback applies to History. The view choice is separate from the frame's Live time switch. Saved layouts from before version 2 drop the retired completed-project retention setting.

## Current

- In progress: the Research page's compact card for each running, paused or unknown-state project, closest to completion first (then the earlier start). The card shows the blueprint, name, scientist portrait and name, the whole-number percent, the iteration count and a progress line. Paused cards say so in the warning colour, and the line turns to it; an unknown state is named and its line is quiet. When nothing is running, one line says so.
- Completed: one-line rows, newest first, with the completion date shown plainly and the last assigned scientist. Projects the recording first saw already complete fold into one row, "N completed by" the latest of their first reads, which opens to their names.
- Cancelled: muted rows with the cancellation date and the progress last seen.
- The shared tooltip adds the type, the last-assignment date when it differs from the completion, the start, "by" bounds and the game's own iteration text. Right-click, Enter or Shift+F10 offers Jump to the start and completion.
- Several countries compare in side-by-side panels. A panel requests only its holder at the shown time.

## History

- One lane per project with a recorded run in the shown range, in start order. Each bar splits at every recorded progress step, one segment per iteration, labelled with its percent where it fits. Running segments take the timeline's shade, deeper as progress rises. Completed bars keep the positive fill with the tick at the game's completion date. Paused segments are neutral and cancelled lanes muted.
- Projects that ended before the range, including those already complete when first recorded, get no lane. One closing line counts them by the latest of their end bounds.
- Lanes follow recorded aliases and group membership, and missing identity stays unknown. Several countries get one heading each.

## Notable behaviour

- While preparation runs, a shown hour it has not reached shows running and paused cards from that hour?s frame and completed/cancelled projects through the prepared range. Start dates remain **Still being prepared**, with their Jump disabled. History bands the pending hours and uses no transient cards. The time axis and Still being prepared key remain visible when the prepared prefix has no project lanes. Starts are observation bounds ("by"). Recognized native completion history dates a completion exactly; otherwise it is bounded by the read. A future completion stays hidden until observed. Progress is stepped and gaps never fill missing hours.
- Iteration counts are the native completed-history count, without a denominator. The game's localized count text appears in the tooltip. Country breakthrough points are not attributable to a project.
- Completion clears the scientist reference, so a completed project names the scientist from an earlier observation in that episode and the tooltip dates it. Gaps cannot supply attribution. Missing portraits and blueprints use the helmet and flask fallbacks.

## Files

| File | Responsibility |
|---|---|
| [SpecialProjectsWidget.tsx](SpecialProjectsWidget.tsx), [specialProjectsData.ts](specialProjectsData.ts) | Current sections, History lanes, validation and dated lookups; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [specialProjects.css](specialProjects.css) | Settings and migration, registration, and sizes for the shared National focus cards and rows. |
