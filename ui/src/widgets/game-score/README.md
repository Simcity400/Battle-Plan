# Game score

## What it shows

The game’s current country score getters and multipliers as a table or Total-score race.

## Products it reads

[`game_score`](../../../../crates/dashboard/src/products/game_score.rs) supplies scores, identity and factions. [`country-flags`](../../../../crates/dashboard/src/products/country_flags.rs) supplies flags at the exact row hour.

## Settings

All/manual/group/Top N countries; campaign/follow/pin, table/race, faction sorting, Top N measure, score parts, multiplier precision and shared table controls. Table defaults to Land descending.

## Notable behaviour

- Scores may rise or fall and never accumulate; branch end is simply the score at that hour. Industry and Land are separate from Total.
- Sort exact raw integers with missing values last. The displayed Score multiplier is Total’s; tooltips retain every multiplier and its supplying date. Content-evaluated values have no recorded date.
- Exact-hour native identities/names/colours own row/race looks. Known empty names remain empty; missing identity or source changes cannot retain an earlier holder’s flag.
- Table/race keys follow native holders through renames. Name, colour and faction changes preserve score easing; identity changes/gaps end it. Tag reuse gets a new bar; dated looks and Jump retain separate tags/hours.
- Plain column labels use recorded end-screen localisation for hints. Jump uses the row hour; fitted tooltips retain full accessible detail.

## Files

| File | Responsibility |
|---|---|
| [GameScoreWidget.tsx](GameScoreWidget.tsx) | Table/race presentation and dated looks. |
| [gameScoreData.ts](gameScoreData.ts) | Validation, exact scores/sorting and tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings, registry choices and registration. |
