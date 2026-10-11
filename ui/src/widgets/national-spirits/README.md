# National spirits

## What it shows

The country’s current national spirits as game icons in acquisition order; campaign mode shows the branch-end state.

## Products it reads

[`idea-history`](../../../../crates/dashboard/src/products/idea_history.rs) through [useIdeaHistory](../../dashboard/ideaHistory.ts), with [game-style idea tooltips](../../dashboard/useIdeaTooltips.ts).

## Settings

Spirit names on hover (default) or under icons; country and campaign/follow/pin. Follow is the default.

## Notable behaviour

- Spirits come from every content category whose type is national_spirit; do not select a fixed category token.
- Missing pictures use a sparkle icon in the tile; the name and keyboard-accessible tooltip remain available.
- Held before means acquisition was not observed. Tooltip effects and targeted country names use the recorded acquisition context.
- Exit fades/tooltips belong to source revision, branch, country and holder segment; changing context clears them.
- Jump uses the acquisition’s first recorded hour, without claiming an exact start for Held before. Shared spiritsData also serves [Spirit history](../spirit-history/README.md).

## Files

| File | Responsibility |
|---|---|
| [NationalSpiritsWidget.tsx](NationalSpiritsWidget.tsx), [spiritsData.ts](spiritsData.ts) | Tiles, scoped exits and shared spirit spans; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [nationalSpirits.css](nationalSpirits.css) | Settings, registration and layout. |
