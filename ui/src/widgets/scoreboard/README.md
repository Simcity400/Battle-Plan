# Scoreboard

## What it shows

A moment table comparing selected country statistics; it has no combined score.

## Products it reads

[`scoreboard`](../../../../crates/dashboard/src/products/scoreboard.rs) owns the column registry and reuses individual products’ measures. Its prepared adapter reads an index of identities and sort values at the displayed hour, then supplies other cells for the shared table's mounted rows. Shared dated country facts supply names and flags.

## Settings

Follow or Pin, columns, countries/groups/Top N, ranking measure, exact sorting and shared table presentation. Default selection is Top 10.

## Notable behaviour

- Keep exact decimal values and supplying dates; missing values are dashes, never zero. Native country keys survive tag changes; ties use current tags.
- Sorting Country alphabetises the numerical Top N; its ranking measure is separate. One request retains all country/column choices, subject only to the shared reply budget.
- Casualties are signed land/naval/air counters. Taken excludes losses without a recorded attacker. Legacy atLeast displays a partial total, not a mathematical bound because omissions can include corrections.
- Use the shared table clock and viewport rows. This table defines no app context-menu actions; selectable text retains the browser menu.
- Each prepared query reads one moment. Exact bar maxima cover the whole selected table; scrolling does not change their scale. Pending moments retain their own supplying hour, and source or selection changes discard that presentation.
- Once the table reports mounted rows at the index hour, cell requests contain only those rows. Before that report, the first page retains matching visible holders and leading ranks.
- A completed cell reply must cover its requested countries. A failed or incomplete cell reply keeps its error across index publications until Retry or a change in client, source, hour, selection or wanted countries. Reordering the same wanted countries does not retry.
- The parity table is labelled `Scoreboard`; its preparation hook loads all selected cells for comparison.

## Files

| File | Responsibility |
|---|---|
| [ScoreboardWidget.tsx](ScoreboardWidget.tsx) | Registry/value requests and shared table. |
| [scoreboardData.ts](scoreboardData.ts) | Exact columns, rows, identities and sorting; adjacent tests. |
| [preparedScoreboard.ts](preparedScoreboard.ts) | Prepared moment/page reads, exact maxima and pending presentation; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [scoreboard.css](scoreboard.css) | Options, content choices, registration and layout. |

The default index can show exact frame factory ranks before preparation. Its
mounted cell page supplies each covered column at the requested hour and leaves
other columns' values absent with preparing keys. Those cells and tooltips say
Still being prepared; no maximum is invented for them. Index and cell requests
publish through the shared coverage loop until the suppliers answer exactly.
Cold rows do not claim that a country or its statistics were not recorded.
