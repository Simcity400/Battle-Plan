# Most decorated divisions

## What it shows

Every chosen country's divisions in service at the shown date, ranked by their recorded medal total, with their awards, experience level and the game's kill counter. A row opens that division's service record.

## Products it reads

[`division-careers`](../../../../crates/dashboard-army/src/products/division_careers.rs) at the shown hour; the drill reads `division-record` as the [Division service record](../division-service-record/README.md) does.

## Settings

Countries (all by default); Campaign (default), Follow or Pin; Sort by (Medals, Experience, Enemy soldiers killed) and the shared table options; drill placement.

## Notable behaviour

- Medals are the history's recorded total, never a sum of award entries; awards show as name chips with per-medal counts in the tooltip. An unread history is "—", never 0.
- Battles won or fought are not recorded per division and are not shown; the kill counter is the game's own.
- Experience shows the level's in-game name with its progress as the cell bar.
- The drill is the same record body as the Division service record widget, with Follow and Pin as widget.
- Only mounted rows ask for country names and flags; thousands of divisions stay one windowed table.

## Files

[MostDecoratedWidget.tsx](MostDecoratedWidget.tsx) owns the table and drill; shared data and the record view live in [division-service-record](../division-service-record/README.md).
