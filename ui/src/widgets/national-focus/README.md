# National focus

## What it shows

Ordered focus completions plus the current/unfinished focus, with recorded progress and conditional game artwork.

## Products it reads

[`national-focus`](../../../../crates/dashboard/src/products/national_focus.rs) supplies order/progress; [`focus-artwork`](../../../../crates/dashboard/src/products/focus_artwork.rs) supplies dated names, descriptions and conditional icons.

## Settings

Country and campaign/follow/pin; Whole campaign is the default. There are no widget-specific options.

## Notable behaviour

- Newly observed completions say By their confirming read; entries already in the first readable snapshot have no known date. Repeats retain separate ordinals. No future outcome fills a gap.
- Progress uses recorded work/base duration. Days left require a previous daily gain of the same focus; do not estimate through gaps. Progress-budget refusal leaves ordered completions usable.
- Artwork is dated to the shown or confirming hour, scoped to client/source/branch/country/content. Pending conditional art never keeps an older choice; defined empty names stay empty.
- Dated completed rows offer Jump to their confirming read; undated rows do not. Enter/Space toggle details. Batch all wanted artwork without truncating the list.

## Files

| File | Responsibility |
|---|---|
| [NationalFocusWidget.tsx](NationalFocusWidget.tsx) | Current card, completion rows and interactions. |
| [focusData.ts](focusData.ts), [useFocusArtwork.ts](useFocusArtwork.ts) | Validation/local lookups and dated artwork requests; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [nationalFocus.css](nationalFocus.css) | Metadata, registration and layout. |
