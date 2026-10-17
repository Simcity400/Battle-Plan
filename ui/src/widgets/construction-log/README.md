# Construction log

## What it shows

One row per observed construction project. Each row has, after its date:

- the building's art and name with its levels, over where it is;
- the country, unless the widget shows exactly one;
- a status pill;
- while the project is in the queue, the factories on it.

## Products it reads

[`construction-log-manifest` and `construction-log`](../../../../crates/dashboard-construction/src/products/construction_log.rs) supply prepared branch windows and building types, then each window's project histories and factory output. Project-keyed events retain later operands without replacing earlier window pages. [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) supplies dated state labels through the shared naming hook.

## Settings

- **Countries:** all countries by default, or manual countries/groups.
- **Building types:** a searchable list with the game's art.
- **Projects:** new and repairs, new buildings or repairs.
- **In the queue:** off by default.
- **Order:** latest change or start.
- **Shared log options.**

Campaign/follow/pin/range are available. With Filter chips on, one line above the log holds the Projects chips, the In the queue chip and the building-type picker, sharing the settings' selections.

## Notable behaviour

- **Row layout.**
  - Rows appear at the read establishing them. A changed project re-enters at its latest change; Jump offers start/change/end dates.
  - The building and its levels sit over the place (a railway "A to B"; an unnamed place capitalised, game names untouched), then the country.
  - From 520 px down, the country joins the line under the building. Compact density puts the place after the building on one line.
- **Art.** Building art uses the campaign's content; a factory icon fills a missing picture slot. The game's repair frame marks a repair over its art.
- **Status pill.**
  - Accent while building or repairing; it pulses during Live.
  - Positive once finished.
  - Neutral for removed, left the queue and not recorded.
  - In the queue keeps projects still building, or unread since a gap, at the shown hour.
- **Factories.**
  - Shown only while the project is in the queue, with "—" between a gap and the next read.
  - An ended project's tooltip gives the factories at its last read.
- **Level counts and endings.**
  - Level counts retain ranges and earlier-work lower bounds. Repair/build phases can share one project; gaps leave status and factories unread. Sampled speed does not bound work between reads.
  - Finished requires a native completion outcome. Complete unlinked departures can be Removed; uncovered departures remain Left the queue. Building rises and factory output do not prove completion.
- **IC amounts.**
  - Signed factory IC excludes bank withdrawals, instant progress and railway carry.
  - The tooltip writes cost per level (a railway's route cost) and factory output in whole IC; the recorded amounts stay exact.
  - Between-read projects without owner/target remain unidentifiable.
- **Names.**
  - Building and state titles preserve successfully localised empty text and titles equal to their keys. Missing titles stay unavailable; diagnostic details retain their keys.
  - Manual tags follow native holders/aliases; groups match at first read. Missing identity stays unknown.
  - Each drawn row names its country at its first read: every drawn hour is requested, beyond the default hour limit. A name still being read shows a placeholder; a missing or failed one says it is unavailable.
  - [useStateNamesAt](../../dashboard/useStateNamesAt.ts) resolves shown-hour states: older uncaptured fields use authored fallback; failed names keep an unavailable label and a concise status.
- **Windows.** One window per replay block lists every project with a row date in it.
  - The widget holds the shown hour's window, the next during Live and, back from there, as many rows below the drawn ones as are drawn. Short of them it reaches further back, filters included.
  - Rows dated before the oldest window held unbroken wait for it, so the list is always the whole log's newest rows.
  - A far jump keeps the last shown hour until its window arrives.
  - Before preparation reaches the shown hour, the log shows its prepared prefix's last hour with "Prepared A to B; the rest is still being prepared." The manifest lists the building types prepared so far, and each window held is replaced as the prefix grows, since its projects' later history is still being prepared.

- Parity builds expose the reached history boundary to the [browser harness](../../../e2e/README.md), so a stable newest window cannot certify a complete log. Ordinary builds expose no test metadata.
- After normal visible screenshots, parity builds prepare every row through the original history reach, requiring the real history boundary. Complete models retain every ordered project operand, referenced building title and exact dated country/place reference. Complete-history preparation requests no offscreen names or artwork.

- Parity builds expose the reached history boundary to the [browser harness](../../../e2e/README.md), so a stable newest window cannot certify a complete log. Ordinary builds expose no test metadata.
- After normal visible screenshots, parity builds prepare every row through the original history reach, requiring the real history boundary. Complete models retain every ordered project operand, referenced building title and exact dated country/place reference. Complete-history preparation requests no offscreen names or artwork.

## Files

| File | Responsibility |
|---|---|
| [ConstructionLogWidget.tsx](ConstructionLogWidget.tsx), [useConstructionLog.ts](useConstructionLog.ts) | Filters, row cells and visible-row names; products and the windows held. |
| [constructionData.ts](constructionData.ts) | Validation, window merging and reach, project bounds, queue/status/factories, row wording and name requests; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [constructionLog.css](constructionLog.css) | Registration, settings, row grid and narrow/compact layouts. |

The log requests a newest-window proof separately while already showing its prefix.
A proven window shows the requested newest hour with a "Newest days exact" note.
Its own building dictionary is remapped by recorded identity when windows merge.
The look-back must include each selected project's real first queue read and its
output-chain start; long projects extend it. Building spellings must already be
resolved in the forward catalogue. Unreadable or unproven windows leave the prefix
shown. Prefix rows say Approximate until complete. A proven numeric newest window
is shared across re-asks without scanning again.

A cold empty manifest is Still being prepared, rather than an assertion that
construction was not recorded. Its ordinary progressive request supplies the
first types and projects as the family publishes.
