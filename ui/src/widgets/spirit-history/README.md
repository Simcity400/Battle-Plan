# National spirits history

## What it shows

One timeline lane per national spirit, showing observed holdings and whether their start/end is exact or bounded.

## Products it reads

[`idea-history`](../../../../crates/dashboard-base/src/products/idea_history.rs) through [useIdeaHistory](../../dashboard/ideaHistory.ts), sharing [spiritsData.ts](../national-spirits/spiritsData.ts) with National spirits.

## Settings

Country, campaign/follow/pin/range; sort by gained/ended/name, show ended spirits, and Grow with playback. Follow is the default.

## Notable behaviour

- All content categories of type national_spirit count. Names, icons and effects use the recorded stack.
- Held before, Ended by and Last recorded preserve uncertain boundary meanings; gaps never imply an exact acquisition or removal. Continuing holdings can join across gaps without filling unread hours.
- Grow changes the axis while preserving zoom and recorded spans; Campaign/Range retain full extent. The shared lane timeline owns navigation, tooltips and dated Jump.

## Files

| File | Responsibility |
|---|---|
| [SpiritHistoryWidget.tsx](SpiritHistoryWidget.tsx) | Spirit lanes and game-style tooltips. |
| [definition.ts](definition.ts), [index.ts](index.ts), [spiritHistory.css](spiritHistory.css) | Settings, registration and layout. |
