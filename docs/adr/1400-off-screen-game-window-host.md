# Host native game windows as off-screen bitmaps with region tables

Game windows opened from dashboard widgets (equipment designers, the military industrial organisation window, event popups, the intelligence agency upgrade window, and — once its stats are verified — the division designer) are rendered off-screen into a PNG bitmap paired with a logical-coordinate region table, rather than hosted in a separate child HWND.

## Decision

- Render each requested game window on demand through a headless `wgpu::Device` using the existing `viewer-native-gui` layout and shader pipeline (`SpritePass`), cropping to the window's drawn extent rather than its declared frame size so `clipping = no` borders are preserved.
- Sample animations at a steady-state tick (`time = 10.0, dt = 0.0`) so button sheen effects do not capture a mid-sweep highlight frame.
- Render at the page's device pixel ratio (`0.5..=3.0`, covering 1, 1.25, 1.5, and 2) so text and sprites stay crisp without browser scaling.
- Return a region table in logical window coordinates carrying authored tooltips (with disabled fallback), close-button hit targets, and scroll containers. The React host (`GameWindowView`) routes hover tooltips through the shared `TooltipLayer` popover, dispatches close clicks, and updates scroll offsets without adding a native child window or cutting holes in the page.
- Cache loaded interface definitions, fonts, textures, and compiled `SpritePass` state per `(source_id, revision, content_folders, visual_revision, window)`, loading only `assets_in(...)` for the requested window. Evict cached entries when a recording is closed (`dashboard_release_source`) or when content folders or visual mods change. Keep zero background threads and zero GPU/CPU work while idle.
- Consolidate game-window registration in `ui/src/dashboard/gameWindows.ts` (replacing the separate `designerSnapshot.ts` hook). Widgets expose a drill action only once a window kind is registered; missing or unsupported interface windows and mods fail with a named refusal.

## Why not a child HWND

A separate child window would require duplicating the native map's hole-cutting, z-ordering, popover occlusion, and DPI resize coordination for every inline drill and pop-out panel, and would keep a live swapchain while idle. An off-screen bitmap integrates directly into inline drills, pop-out popovers, and scroll containers with zero idle GPU cost and exact `viewer-native-gui` shader parity.

## Visual differences and later list layout

- **3D preview box**: Designer windows keep the game's framed 3D preview box as a labelled empty box rather than adding a 3D model renderer.
- **Simplified list ordering**: When the Equipment Designs and Division Templates widget lists are simplified in a follow-up PR, live entries appear first in the game's own order, followed by a separate `"Deleted"` section showing each deleted entry's deletion date.

[Off-screen renderer](../../crates/viewer/src/game_windows/mod.rs) · [Desktop command](../../crates/desktop/src/game_windows.rs) · [React host](../../ui/src/dashboard/GameWindowView.tsx) · [Game window registry](../../ui/src/dashboard/gameWindows.ts) · [Native map and game window ownership](0504-native-map-and-game-window-ownership.md)
