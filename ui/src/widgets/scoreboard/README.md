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
- A completed cell reply must cover its requested countries. An incomplete reply stops with an error until retried.
- The parity table is labelled `Scoreboard`; its preparation hook loads all selected cells for comparison.

## Files

| File | Responsibility |
|---|---|
| [ScoreboardWidget.tsx](ScoreboardWidget.tsx) | Registry/value requests and shared table. |
| [scoreboardData.ts](scoreboardData.ts) | Exact columns, rows, identities and sorting; adjacent tests. |
| [preparedScoreboard.ts](preparedScoreboard.ts) | Prepared moment/page reads, exact maxima and pending presentation; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [scoreboard.css](scoreboard.css) | Options, content choices, registration and layout. |
