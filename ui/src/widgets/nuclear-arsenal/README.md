# Nuclear arsenal

## What it shows

The game’s whole nuclear stockpile counter as a race, total lines or one-country type mix, with a native-holder type drill.

## Products it reads

[`country-nukes`](../../../../crates/dashboard/src/products/country_nukes.rs) supplies counters, type runs and independent read/identity histories; shared country facts supply dated looks.

## Settings

Top 10 by default, manual/groups; campaign/follow/pin/range, race/lines/mix, date/end labels, playback/axes and Show/drill choices.

## Notable behaviour

- Uses native whole-counter arithmetic, not ready queued drops. Content/dictionary names identify types. Top N ranks nonzero existing countries; manual known zero stays zero.
- Read dates stay independent of compressed values. Missing rows/tables/identity and unknown totals break history and easing; no gap becomes zero.
- Picks bind the native holder at the opening date through its aliases, excluding replacement holders. Overlap/keyboard choices hold the clicked hour until selection.
- Manual tags select every recorded native holder under those tags and its dated aliases; backend requests keep literal picks. Smart groups use dated membership. Follow converts the clock through track alignment.
- Drills retain signed type counts, unknown ordinals and exact whole-counter residual separately. Picking holds; Follow uses the real clock even from a held main view. Pin saves original actor/date; its country chip follows the displayed drill date.
- Same-request NumericReply survives content-only refresh; source/selection/numeric changes mask stale picks immediately.

## Files

| File | Responsibility |
|---|---|
| [NuclearArsenalWidget.tsx](NuclearArsenalWidget.tsx), [NuclearCountryChip.tsx](NuclearCountryChip.tsx) | Shared race/drill and dated actor header. |
| [nuclearArsenalData.ts](nuclearArsenalData.ts) | Validation, exact counts, actor-bound histories and pins; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Options and registration. |
