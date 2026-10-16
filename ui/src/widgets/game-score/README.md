# Game score

## What it shows

The game’s current country score getters as a table or Total-score race.

## Products it reads

[The game-score prepared adapter](../../../../crates/dashboard/src/products/game_score/prepared.rs) supplies one hour of scores, identity and factions through shared prepared-series queries. Beyond accepted coverage it reconstructs that hour's replay block directly. [`game_score`](../../../../crates/dashboard/src/products/game_score.rs) supplies the whole-branch reference history. [`country-flags`](../../../../crates/dashboard/src/products/country_flags.rs) supplies flags at the exact row hour.

## Settings

All/manual/group/Top N countries; campaign/follow/pin, table/race, faction sorting, Top N measure, score parts and shared table controls. Table defaults to Land descending.

## Notable behaviour

- Each table/race request draws immediately, including countries present only beyond prepared coverage. Direct-hour replies are marked approximate and carry no prior race continuity. Accepted progress replaces them through the shared preparation helper; idle or failed preparation retains drawn rows. A pending hour retains the previous reply at its own displayed date. Source, branch and content changes revoke that display.
- Scores may rise or fall and never accumulate; branch end is simply the score at that hour. Industry and Land are separate from Total.
- Sort exact raw integers with missing values last. The score multipliers are game-wide values, the same for every country, so they are not a column: tooltips show every multiplier once, without raw integers or supplying dates.
- Exact-hour native identities/names/colours own row/race looks. Known empty names remain empty; missing identity or source changes cannot retain an earlier holder’s flag.
- Table/race keys follow native holders through renames. Name, colour and faction changes preserve score easing; identity changes/gaps end it. Tag reuse gets a new bar; dated looks and Jump retain separate tags/hours.
- Plain column labels use recorded end-screen localisation for hints. Jump uses the row hour; fitted tooltips retain full accessible detail.
- Faction names and localised hints preserve known empty text and titles equal to their keys; missing inputs retain their reasons.
- Parity builds expose every ordered table row, product-owned identity and exact cell operand. Visible text and screenshots cover mounted cells and artwork. The widget also exposes the "Game score values" parity model for both views. Ordinary builds expose no row model.

## Files

| File | Responsibility |
|---|---|
| [GameScoreWidget.tsx](GameScoreWidget.tsx) | Table/race presentation and dated looks. |
| [usePreparedScore.ts](usePreparedScore.ts) | Progressive whole-hour loading, preparation refresh and source ownership. |
| [gameScoreData.ts](gameScoreData.ts) | Validation, exact scores/sorting and tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings, registry choices and registration. |
