# National spirits history

## What it shows

One timeline lane per national spirit, showing observed holdings and whether their start/end is exact or bounded.

## Products it reads

[`idea-history`](../../../../crates/dashboard-base/src/products/idea_history.rs) through [useIdeaHistory](../../dashboard/ideaHistory.ts), sharing [spiritsData.ts](../national-spirits/spiritsData.ts) with National spirits. Before preparation reaches the shown hours, the lanes combine its exact prefix with keyframe holdings beyond it. Those spans use the faded dashed approximate style and the **Approximate** key. Sample edges never claim acquisition or removal dates; tooltips name their dates still being prepared. Unsampled hours retain the **Still being prepared** band. Gate coverage replaces the sampled spans with exact hourly holdings. A holding reaching an unprepared boundary has no claimed ending.


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
