# Scoreboard

## What it shows

A moment table comparing selected country statistics; it has no combined score.

## Products it reads

[`scoreboard`](../../../../crates/dashboard/src/products/scoreboard.rs) owns the column registry and reuses individual products’ measures. Shared dated country facts supply names and flags.

## Settings

Follow or Pin, columns, countries/groups/Top N, ranking measure, exact sorting and shared table presentation. Default selection is Top 10.

## Notable behaviour

- Keep exact decimal values and supplying dates; missing values are dashes, never zero. Native country keys survive tag changes; ties use current tags.
- Sorting Country alphabetises the numerical Top N; its ranking measure is separate. One request retains all country/column choices, subject only to the shared reply budget.
- Casualties are signed land/naval/air counters. Taken excludes losses without a recorded attacker. Legacy atLeast means Recorded subtotal, not a mathematical bound because omissions can include corrections.
- Use the shared table clock and viewport rows. This table defines no app context-menu actions; selectable text retains the browser menu.

## Files

| File | Responsibility |
|---|---|
| [ScoreboardWidget.tsx](ScoreboardWidget.tsx) | Registry/value requests and shared table. |
| [scoreboardData.ts](scoreboardData.ts) | Exact columns, rows, identities and sorting; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [scoreboard.css](scoreboard.css) | Options, content choices, registration and layout. |
