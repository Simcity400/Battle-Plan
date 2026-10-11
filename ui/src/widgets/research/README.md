# Research

## What it shows

Research-slot histories and current assignments, with recorded progress, days-left estimates and an expected/took time explanation.

## Products it reads

[`research`](../../../../crates/dashboard/src/products/research.rs) supplies slot states, journeys, modifier credits and localized text; playback resolves them locally. [`technology-artwork`](../../../../crates/dashboard/src/products/technology_artwork.rs) loads images in small independent batches, so history and titles do not wait for rendering. Hovering a title reveals its technology key.

## Settings

Country and campaign/follow/pin; Whole campaign is the default. No widget-specific options.

## Notable behaviour

- Dates are observation bounds. A later technology-inventory read can confirm completion, but the row stays hidden until confirmation. Jump retains the recorded ending bound.
- The product’s sliced observations govern progress/modifiers; modifier-only changes wait for the next country read. Never invent fresher samples or interpolate through gaps.
- Expected/took attribution keeps saved-day juggling, prior work, unfinished work, out-of-slot time, minimum-speed recovery and unexplained residuals separate. Additive bonuses are not independent multiplicative discounts.
- Current-card Jump uses the earliest joined assignment observation, including earlier slots. No pre-recording start is inferred. Enter/Space open details.
- Diagnostics update when their text changes and clear on unmount; equal warnings do not restart a render cycle while images load.
- Failed image requests report a counted diagnostic while successful images and independent history titles remain available. Source or content changes clear those image results and failures.
- Image batches follow the displayed cards, prioritising current assignments before completed rows. Live and pinned moments request no future technologies; repeated technologies share one image.

## Files

| File | Responsibility |
|---|---|
| [ResearchWidget.tsx](ResearchWidget.tsx), [researchData.ts](researchData.ts) | Slot cards, rows and local lookups. |
| [researchJourney.ts](researchJourney.ts), [researchBreakdown.ts](researchBreakdown.ts) | Joined assignments and exact time/modifier parts; adjacent tests. |
| [ResearchExplanation.tsx](ResearchExplanation.tsx), [researchTimeBar.ts](researchTimeBar.ts) | Shared explanation rows and segment presentation. |
| [definition.ts](definition.ts), [index.ts](index.ts), [research.css](research.css) | Metadata, registration and layout. |
