# Frontend

The React/TypeScript interface hosted by the desktop app. It owns navigation, appearance, saved layouts, the shared playback clock and the widget grid. The [viewer](../crates/viewer/README.md) draws native maps into widget bodies.

## Layout

| Area | Responsibility |
|---|---|
| [src/shell/](src/shell/) | App views, Settings, appearance, Tauri API wrappers and native viewer lifecycle. |
| [src/components/](src/components/) | Shared popovers, tooltips, icons, country flags and pickers, playback and map controls. |
| [src/dashboard/](src/dashboard/) | Campaigns, layouts, widget frames/settings, playback, data clients and product caches. |
| [src/charts/](src/charts/README.md) | Shared Canvas charts, HTML tables, motion, exact formatting and interactions. |
| [src/widgets/](src/widgets/README.md) | Widget registration and one folder per widget. |
| [src/generated/](src/generated/) | Rust-generated TypeScript contracts; regenerate rather than edit. |
| [styles.css](src/styles.css) | Global design tokens and common controls. Component CSS refines these variables. |

## Development

Run `npm.cmd run dev:ui` from the repository root for Vite only. `check:ui`, `test:ui` and `build:ui` are the frontend checks/build; the tests live beside their modules and need no game. Desktop startup and Rust build instructions are in the [desktop guide](../crates/desktop/README.md) and [AGENTS.md](../AGENTS.md).

### Real recordings in the browser

Build the dashboard's `dashboard-bridge` binary with `cargo build -p dashboard --bin dashboard-bridge` from the repository root. Set `BATTLE_PLAN_RECORDINGS_DIR` in ignored `ui/.env.local` to a scratch folder outside Git; `BATTLE_PLAN_GAME_DIR` and `BATTLE_PLAN_USER_DIR` enable artwork. Settings folders take precedence. [browserBridge.ts](browserBridge.ts) starts the adapter on demand and forwards cancellation. Production uses the desktop transport. Both transports enforce the generated message-size budget and preserve country selections intact.

## Data and identity

The [dashboard architecture](../docs/dashboard.md) and [Rust dashboard guide](../crates/dashboard/README.md) describe the product boundary. [client.ts](src/dashboard/client.ts) defines the frontend client; [nativeClient.ts](src/dashboard/nativeClient.ts) and [browserClient.ts](src/dashboard/browserClient.ts) implement it.

- [products.ts](src/dashboard/products.ts) shares identical requests through `useProduct`/`useProducts`. Full-branch histories support local playback lookups; hour-specific facts use `loadTransientProduct` so they do not evict histories. Closing a source releases its products and list flags.
- [productWindows.ts](src/dashboard/productWindows.ts) bounds dense histories by requested windows. Hold the visible/next playback window and reached tooltip windows, then release unwanted demand. `useWindowDisplay` retains a loading window at its own hour; pass the client to bind that display to its source owner.
- Hook data, errors and callbacks belong to the client and request identity. A source switch must mask old results immediately. [contentRevision.ts](src/dashboard/contentRevision.ts) invalidates content-dependent products and artwork when content changes; recording-only histories remain independent.
- [numericReply.ts](src/dashboard/numericReply.ts) retains same-request numeric history while labels reload; keep its coverage, errors and pick ownership independent of artwork. [countryIdentity.ts](src/dashboard/countryIdentity.ts) preserves native rows across tag changes without filling identity gaps.
- Validate payloads with [validation.ts](src/dashboard/validation.ts). Keep recorded integer/decimal-string operands exact until formatting; floating point is for geometry. Preserve zero, missing values, refusal reasons, coverage gaps and supplying read hours separately.
- Dated names, flags and colours use the shared country/faction helpers. Retained looks apply only while the next read loads within the same identity and recorded stretch; a completed missing read stays unknown. Request flags only for drawn countries. See [looks.ts](src/dashboard/looks.ts), [countryLooks.ts](src/dashboard/countryLooks.ts) and [factionNames.ts](src/dashboard/factionNames.ts).

## Layouts and playback

[model.ts](src/dashboard/model.ts) owns templates, tracks and widget time modes; [storage.ts](src/dashboard/storage.ts) saves semantic choices and branch-relative offsets, never runtime handles. Unknown widget kinds or unsupported settings retain their saved data. An unstorable country choice reports a restoration warning without discarding the layout or rewriting its saved data. Content/country lists have no shared count or token-length cap.

[playback.ts](src/dashboard/playback.ts) is the single clock shared by Overview and Campaign map. Subscribe only when following it; resolve widget windows through the model. Animations read `positionAt` between ticks, while the map receives the latest hour. A widget can follow, show its branch end or pin a moment as its definition allows. Settings and Live changes save immediately; adding, moving, resizing and removing widgets wait for edit-mode Save. Campaign assignment opens a recording through the shared source path and keeps it open until sidebar unload.

Use [calendar.ts](src/dashboard/calendar.ts): the game's fixed 365-day calendar, display hours 01:00 through 24:00, and raw engine hours underneath. Never carry values across unrecorded hours. Partial recordings retain a terminal failure notice above both views and marks in campaign/library entries; an unknown failure hour is never guessed.

## Shared appearance and controls

Use [Popover.tsx](src/components/Popover.tsx) for interactive pop-downs, [Tooltip.tsx](src/components/Tooltip.tsx) with `data-tooltip` for hints, [CountryPicker.tsx](src/components/CountryPicker.tsx) for country choices and [Icons.tsx](src/components/Icons.tsx) for the outline icon family. The country picker windows long lists. Flags come from recorded content at the shown hour; missing or failed flags retain a grey placeholder and their reason. The top bar uses the recorded native shield sprite; list flags use the shared medium/large fallback.

Themes, fonts, chart motion, country-colour adjustment and backdrops belong in [appearance/](src/shell/appearance/). Theme roles drive the entire interface through CSS variables; preserve the clean shared style. Default typography is the system UI font at 14px, using weights 400, 500 and 600; Inter and installed fonts remain selectable. Outlined fields and controls use `--control-*`; sidebar rows and widget-header buttons stay borderless. The native map sits above the page: menus hide it while open, and hints stay within their sidebar/header area.
