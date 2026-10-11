# Special projects

## What it shows

Current prototype cards or project-history lanes, with native progress, scientist attribution and iteration counts.

## Products it reads

[`special-projects`](../../../../crates/dashboard/src/products/special_projects.rs) supplies project observations, completion history, content labels and recorded portraits.

## Settings

Countries/groups, campaign/follow/pin/range; Current/History view in the header, completed-card retention and Grow with playback. The view choice is separate from the frame’s Live time switch.

## Notable behaviour

- Current shows a panel for each selected country and requests only its matching aliases at the shown time. History compares holders through recorded aliases and group membership. Missing identity stays unknown.
- Starts say By; recognized native completion history can be exact. Future completion remains hidden until observed. Progress is stepped; gaps never fill missing hours. Paused/cancelled episodes keep their observed meanings.
- Iterations are the native completed-history count formatted with content text, without a denominator/dots. Country breakthrough points are not attributable to a project.
- Completion clears the scientist reference: Last assigned uses an earlier observation in that episode and dates it. Gaps cannot supply attribution. Missing portraits use a helmet fallback; missing count text says Iteration count unavailable.
- Jump offers the bounded start and completion dates. Grow preserves zoom and recorded spans; Pin/Range retain held bounds.

## Files

| File | Responsibility |
|---|---|
| [SpecialProjectsWidget.tsx](SpecialProjectsWidget.tsx), [specialProjectsData.ts](specialProjectsData.ts) | Cards/lanes, validation and dated lookups; adjacent tests. |
| [IterationRow.tsx](IterationRow.tsx) | Native count-to-text rendering. |
| [definition.ts](definition.ts), [index.ts](index.ts), [specialProjects.css](specialProjects.css) | Settings, registration and layout. |
