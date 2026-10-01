# Frontend

The React/TypeScript interface hosted by the desktop app. It owns navigation, appearance, saved layouts, the shared playback clock and the widget grid. Native maps are drawn by the viewer process into widget bodies; do not recreate them here.

## Folders

| Folder | What belongs here |
|---|---|
| [src/shell](src/shell/) | Sidebar, top bar and views, Settings and its sidebar navigation, native viewer glue and the Tauri API wrappers. [appearance](src/shell/appearance/) holds themes, backdrops and fonts. |
| [src/components](src/components/) | Reusable tiles: popover, tooltip, icons, country picker and flag, date picker, playback speed, map controls and appearance. |
| [src/dashboard](src/dashboard/) | The widget framework: layout model and storage, playback clock, sources and campaigns, map geometry, the data client with its two transports, and the product cache. |
| [src/charts](src/charts/) | The Canvas 2D chart kit: one animation scheduler, sharp sizing under zoom, theme tokens and palette, time and value axes, zoom and pan, series and labels. See its [README](src/charts/README.md). |
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

`dev:ui` starts only Vite. The wrapped desktop development command also builds the viewer and starts Tauri; see the [desktop guide](../crates/desktop/README.md). Tests are pure node tests beside their modules; they need no browser, game or recording.

### Real recordings in the browser

Build the adapter once using the shared [build commands](../AGENTS.md#build-commands):

```powershell
bash tools/build-slot.sh "dev/dashboard" "cargo build -p dashboard --bin dashboard-bridge"
```

Set `BATTLE_PLAN_RECORDINGS_DIR` to a scratch folder outside Git in ignored `ui/.env.local`; `BATTLE_PLAN_GAME_DIR` and `BATTLE_PLAN_USER_DIR` add real artwork. Folders saved in Settings take precedence. `browserBridge.ts` is a development-only Vite middleware that starts the adapter on demand, forwards data commands and cancels them with the request. Production builds use the desktop transport.

## Data

Widgets read data products through `useProduct` in [products.ts](src/dashboard/products.ts). A product reply covers the whole branch, so playback is a local lookup and the hour tick never crosses the process boundary. Identical requests share one native round trip in a bounded per-client cache; closing a campaign forgets its products. The client contract is [client.ts](src/dashboard/client.ts); [nativeClient.ts](src/dashboard/nativeClient.ts) and [browserClient.ts](src/dashboard/browserClient.ts) are its transports. The desktop host passes replies through as raw JSON; the frontend parses them once.

## Layouts and playback

[model.ts](src/dashboard/model.ts) defines templates, tracks, widget specs and time modes; [storage.ts](src/dashboard/storage.ts) saves a bounded, versioned library of templates with semantic country choices and no runtime handles. [playback.ts](src/dashboard/playback.ts) is the one clock; widgets subscribe to hour changes only when they follow it, and animations read `positionAt` to move smoothly between its ticks. At high speeds a tick skips many hours, so the map is sent only the latest hour and widgets look hours up locally. Overview and Campaign map share the selected country, clock and speed. A widget follows playback, holds its branch end, or pins a moment; the choice is saved per widget. The Live switch and every choice in a widget's settings panel save to the active layout immediately; adding, moving, resizing and removing widgets are edit-mode actions and wait for Save.

Each widget follows the default campaign or one chosen in its gear menu. Choosing a recording that is not open opens it exactly as the sidebar does: it takes a campaign slot, is read once and is shared by every widget and map that uses it, so data products stay cached per campaign rather than per widget. It stays open until it is unloaded from the sidebar.

Use the game's fixed 365-day calendar in [calendar.ts](src/dashboard/calendar.ts), with hours shown as 01:00 through 24:00 and raw engine hours underneath. Keep exact decimal strings until formatting; floating point is for geometry only. Show gaps and unavailable data honestly; never carry a value across missing hours.

## Shared UI

Use [Popover.tsx](src/components/Popover.tsx) for every interactive pop-down and [Tooltip.tsx](src/components/Tooltip.tsx) with `data-tooltip` for hints. Reuse [CountryPicker.tsx](src/components/CountryPicker.tsx) for country choices; it reads the land-owners and country-flags products. Flags are always the recording's own installed content, never drawn or substituted: lists use the game's medium flags, and the top bar shows the country as the game's top bar does (`shield`: the sprite the recorded `interface/topbar.gui` `player_flag` button names) at the playback hour, so cosmetic tags, ideologies and mods change it when they did in the game. A missing flag shows an empty frame. Recordings may hold thousands of countries, so the picker renders only the rows in view and keeps their list flags in [flagCache.ts](src/dashboard/flagCache.ts), apart from the product cache. Icons come from [Icons.tsx](src/components/Icons.tsx), one outline family.

[styles.css](src/styles.css) is the design system: it derives panels, lines, glass, radii, shadows and motion from the theme roles, and styles buttons, fields, switches, sliders, segmented controls, pills and empty states globally. Component stylesheets refine it through its variables, so every theme restyles the whole interface. Buttons, fields and top-bar controls are outlined with the `--control-*` tokens (hairline, faint fill, top highlight); sidebar rows and widget-header buttons stay borderless. The native map is a separate window above the page: only menus hide it while open, and Tooltip keeps sidebar and header hints inside their own area.

## Appearance

A theme is a palette of the roles in [themes.ts](src/shell/appearance/themes.ts) with a dark and/or a light variant; `deriveColors` builds a readable palette from a canvas, accent and glow. The colour scheme follows Windows or is fixed, and each mode can use a different theme. Custom themes are edited in Settings with a live preview, and exported or imported as JSON; [themeFile.ts](src/shell/appearance/themeFile.ts) also converts VS Code colour themes. Preferences are saved in localStorage by [settings.ts](src/shell/appearance/settings.ts), which migrates the previous format. Imported background images are re-encoded and kept in IndexedDB ([backdrops.ts](src/shell/appearance/backdrops.ts)). The default typography follows T3 Code: the system UI font (Segoe UI on Windows) at a 14px base, with weights of 400, 500 and 600 only, because Segoe UI has no intermediate weights. Inter is bundled as a choice, and any installed font can be named for the interface or display text.
