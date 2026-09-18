# Frontend

The React/TypeScript interface hosted by the desktop app. It owns navigation, analytics controls, exact-value display and history charts. The Map tab embeds the native viewer; do not recreate its renderer here.

## Develop and check

Run from the repository root:

```powershell
npm.cmd run dev:ui
npm.cmd run check:ui
npm.cmd run build:ui
```

Frontend-only development is for layout and browser behavior. File picking, replay queries and map control need `npm.cmd run dev`. See the [desktop guide](../crates/desktop/README.md).

Start in [src](src/), particularly [analytics](src/analytics/). Rust-generated data types are described in the [analytics guide](../crates/analytics/README.md). Regenerate them; do not edit the generated TypeScript.

## Values and charts

Keep exact decimal-string values and scales until formatting. Use integer arithmetic for displayed values; JavaScript floating point is only for chart geometry. Distinguish recorded zero from missing, country-absent, truncated and unavailable samples.

The bounded history response may contain first/last samples and minima/maxima for an interval. It does not retain the complete path inside that interval. Break charts across missing coverage and show aggregation honestly. Do not join branches or give carried samples a new acquisition time.

## Date and time controls

Use the game's fixed 365-day calendar, not JavaScript's Gregorian `Date` arithmetic. The displayed hours are **01:00 through 24:00**, not 00:00 through 23:00. Keep raw signed replay hours as the underlying value.

Respect inclusive branch bounds and validate From ≤ To. A gap can lie inside valid bounds; a parent's load hour does not extend a branch's recorded range. For hours before the supported calendar epoch, preserve the raw value and show explicit unsupported-date behavior rather than inventing dates or silently clamping data.

Calendar navigation changes a draft. Applying the selection commits the range; browsing months or years must not issue history queries. Keep the popup compact and its text readable. Keyboard navigation must leave the focused day visible; Escape returns focus to the trigger. Keep one picker open at a time and preserve visible focus when committing a year.

## Requests and navigation

Do not display an obsolete response over a newer request. Use the host's cancellation and file-revision contract, and refuse stale-result map navigation when the file changed. Follow [AGENTS.md](../AGENTS.md) for the full check set.
