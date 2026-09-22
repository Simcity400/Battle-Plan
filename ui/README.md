# Frontend

The React/TypeScript interface hosted by the desktop app. It owns navigation, appearance, saved layouts, the shared playback clock and the widget grid. Native maps are drawn by the viewer process into widget bodies; do not recreate them here.

## Folders

| Folder | What belongs here |
|---|---|
| [src/shell](src/shell/) | App tabs, Settings, theme, native viewer glue and the Tauri API wrappers. |
| [src/components](src/components/) | Reusable tiles: popover, tooltip, icons, country picker and flag, date picker, playback speed, map controls and appearance. |
| [src/dashboard](src/dashboard/) | The widget framework: layout model and storage, playback clock, sources and campaigns, map geometry, the data client with its two transports, and the product cache. |
| [src/widgets](src/widgets/) | One folder per widget. Start with its [README](src/widgets/README.md). |
| [src/generated](src/generated/) | The TypeScript contract generated from Rust. Regenerate; never edit. |

## Develop and check

Run from the repository root:

```powershell
npm.cmd run dev:ui
npm.cmd run check:ui
npm.cmd run test:ui
npm.cmd run build:ui
```

`dev:ui` starts only Vite. `npm.cmd run dev` also builds the viewer and starts Tauri; see the [desktop guide](../crates/desktop/README.md). Tests are pure node tests beside their modules; they need no browser, game or recording.

### Real recordings in the browser

Build the adapter once with `cargo build -p dashboard --bin dashboard-bridge`. Set `BATTLE_PLAN_RECORDINGS_DIR` to a scratch folder outside Git in ignored `ui/.env.local`; `BATTLE_PLAN_GAME_DIR` and `BATTLE_PLAN_USER_DIR` add real artwork. Folders saved in Settings take precedence. `browserBridge.ts` is a development-only Vite middleware that starts the adapter on demand, forwards data commands and cancels them with the request. Production builds use the desktop transport.

## Data

Widgets read data products through `useProduct` in [products.ts](src/dashboard/products.ts). A product reply covers the whole branch, so playback is a local lookup and the hour tick never crosses the process boundary. Identical requests share one native round trip in a bounded per-client cache; closing a campaign forgets its products. The client contract is [client.ts](src/dashboard/client.ts); [nativeClient.ts](src/dashboard/nativeClient.ts) and [browserClient.ts](src/dashboard/browserClient.ts) are its transports. The desktop host passes replies through as raw JSON; the frontend parses them once.

## Layouts and playback

[model.ts](src/dashboard/model.ts) defines templates, tracks, widget specs and time modes; [storage.ts](src/dashboard/storage.ts) saves a bounded, versioned library of templates with semantic country choices and no runtime handles. [playback.ts](src/dashboard/playback.ts) is the one clock; widgets subscribe to hour changes only when they follow it. Overview and Campaign map share the selected country, clock and speed. A widget follows playback, holds its branch end, or pins a moment; the choice is saved per widget.

Use the game's fixed 365-day calendar in [calendar.ts](src/dashboard/calendar.ts), with hours shown as 01:00 through 24:00 and raw engine hours underneath. Keep exact decimal strings until formatting; floating point is for geometry only. Show gaps and unavailable data honestly; never carry a value across missing hours.

## Shared UI

Use [Popover.tsx](src/components/Popover.tsx) for every interactive pop-down and [Tooltip.tsx](src/components/Tooltip.tsx) with `data-tooltip` for hints. Reuse [CountryPicker.tsx](src/components/CountryPicker.tsx) for country choices; it reads the land-owners and country-flags products. All interface CSS uses the semantic theme variables in [theme.ts](src/shell/theme.ts).
