# Host native game windows as off-screen bitmaps with region tables

Game windows opened from dashboard widgets (starting with the equipment designer — ship, tank, plane, and classic upgrade — and later the military industrial organisation window, event popups, the intelligence agency upgrade window, and once its stats are verified, the division designer) are rendered off-screen into a PNG bitmap paired with a logical-coordinate region table, rather than hosted in a separate child HWND.

## Decision

- Render each requested game window on demand through a headless `wgpu::Device` using `viewer-native-gui` layout and `SpritePass` layer rendering (`set_layer(true)` + `draw_layer`, unpremultiplied to straight-alpha RGBA before PNG encoding). Compute bounds from the emitted `Batch` geometry rather than declared frame sizes so `clipping = no` overhangs and text are preserved, trim fully transparent outer border pixels while shifting logical region origins in lockstep, and refuse oversized surfaces or refused GPU draws by name.
- Settle fade transitions (`time = 10.0, dt = 10.0`) and sample the rendered frame at steady state (`time = 10.0, dt = 0.0`) so button sheen effects do not capture a mid-sweep highlight frame.
- Render at the page's device pixel ratio (`0.5..=3.0`, covering 1, 1.25, 1.5, and 2) so text and sprites stay crisp without browser scaling.
- Return a region table in logical window coordinates carrying authored tooltips (with disabled fallback) and close-button hit targets. The React host (`GameWindowView`) routes hover and keyboard-focus tooltips through the shared `TooltipLayer` popover and dispatches close actions without adding a native child window or cutting holes in the page. Dropdowns and other interactive widgets stay out of the host until a consumer needs them.
- List boxes scroll, once, in the host: a window lays its rows out through `ComposedPlan::list_rows`, which places them with the game's own list box and draws its scrollbar, lays out only the rows inside the list, and returns a scroll region (position, range, step). The request carries a position per list; the React host turns a wheel notch into one row's step and asks again, keeping one render in flight per window and following the latest position. The game declares no wheel step for its lists that reproduces its feel, so a notch moves one row. The position is part of the frame cache key.
- Load `.gui` templates directly through `native::Guis` without requiring map-counter windows so total-conversion mods work, share immutable `Arc<Registry>` data across cached windows, and separate lightweight cache/generation locks from heavy interface loading and GPU rendering so `evict_source`, `clear_cache`, and cancellation never stall map actions or source release. Keep zero background threads and zero GPU/CPU work while idle.
- Consolidate game-window registration in `ui/src/dashboard/gameWindows.ts` (replacing the separate `designerSnapshot.ts` hook). Widgets expose a drill action only once a window kind is registered; missing or unsupported interface windows, mods, and folders fail with typed refusals surfaced directly to the user.

## Why not a child HWND

A separate child window would require duplicating the native map's hole-cutting, z-ordering, popover occlusion, and DPI resize coordination for every inline drill and pop-out panel, and would keep a live swapchain while idle. An off-screen bitmap integrates directly into inline drills, pop-out popovers, and scroll containers with zero idle GPU cost and exact `viewer-native-gui` shader parity.

## Visual differences and later list layout

- **3D preview box**: Designer windows keep the game's framed 3D preview box as a labelled empty box rather than adding a 3D model renderer.
- **Simplified list ordering**: When the Equipment Designs and Division Templates widget lists are simplified in a follow-up PR, live entries appear first in the game's own order, followed by a separate `"Deleted"` section showing each deleted entry's deletion date.

[Off-screen renderer](../../crates/viewer/src/game_windows/mod.rs) · [Equipment designer](../../crates/viewer/src/game_windows/equipment_designer.rs) · [Desktop command](../../crates/desktop/src/game_windows.rs) · [React host](../../ui/src/dashboard/GameWindowView.tsx) · [Game window registry](../../ui/src/dashboard/gameWindows.ts) · [Native map and game window ownership](0504-native-map-and-game-window-ownership.md)
