# Idea timeline

## What it shows

One political-screen idea category over time, with lanes for law/designer slots and advisor seats.

## Products it reads

[`idea-history`](../../../../crates/dashboard/src/products/idea_history.rs) through [useIdeaHistory](../../dashboard/ideaHistory.ts); [idea tooltips](../../dashboard/useIdeaTooltips.ts) provide dated effects and targets.

## Settings

Content category, law shading/icons, holder portrait/name, Grow with playback, and campaign/follow/pin/range. Follow is the default.

## Notable behaviour

- Categories, seats, law order and artwork come from recorded content. Picker presets save one generic widget kind with category/title; a custom title survives category changes.
- Active ideas and hired-advisor observations have separate coverage. Advisors use recorded character/role/seat assignments, not an invented ActiveIdeas entry; gaps keep holdings unknown.
- Portrait-only holders without art retain their name. Tooltips and keyboard descriptions share game-style effects.
- Grow changes the axis, not recorded spans; Campaign and Range retain their full extent.

## Files

| File | Responsibility |
|---|---|
| [IdeaTimelineWidget.tsx](IdeaTimelineWidget.tsx), [ideaTimelineData.ts](ideaTimelineData.ts) | Lanes, holder presentation and validation; adjacent tests. |
| [categoryChoices.ts](categoryChoices.ts), [IdeaTimelinePickerItems.tsx](IdeaTimelinePickerItems.tsx) | Content categories and picker presets. |
| [definition.ts](definition.ts), [index.ts](index.ts), [ideaTimeline.css](ideaTimeline.css) | Settings, registration and layout. |
