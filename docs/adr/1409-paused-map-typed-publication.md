# 1409. Typed async publication for paused map wake

Status: Accepted · Date: 2026-11-28

## Context

After ADR 1408 froze ambient animation on paused dashboard map widgets, a paused widget stops requesting frames once settled. Map layers, the overlay worker, row views, combat windows and worker-pool jobs finish asynchronously off the UI thread. Without a wake when an answer lands, a paused map could draw the old picture, fail to advance `rendered_sequence` (leaving the dashboard widget stuck in `loading = true` after drawing the new hour), or stall on a mode switch whose overlay was still building or refused.

Manually calling a global counter at every worker exit site is fragile: any new producer or new error branch that forgets to signal silently recreates the paused-map stall.

## Decision

1. **Enforce async landing by construction in `viewer_runtime::landed`.**
   - `Landed<T>` has a private inner field and no public constructor. It is created only by receiving from a `landed::channel` or reading from `Published<T>` / `PublishedKeyed<K, V>`, whose writes atomically advance a single process-wide `LANDED` counter. `PublishedHour` advances `LANDED` only when the stored hour changes.
   - `Fed<P>`, `Reads<K, T>`, `Rebuilt`, `Whole`, `ShownVision`, `Combat`, `Overlay`, and the async fields of `side::Products` and `side::LayIns` carry `Landed<T>` or publish through `Published` / `PublishedKeyed`. `WorkerPool::try_submit` requires an explicit wake closure.
   - `PublishedKeyed<K, V>::take` preserves the last published key, so idle workers re-answering unchanged data or repeating an identical refusal produce zero `LANDED` increments.
   - `SideThread` rests at a 16 ms interval (`REST`) when paused, settled, and nothing changed (`rests`), waking the UI via its injected wake closure only when a new ask, unsettled layer state, or a `LANDED` / view-demand change requires a turn. Identical live asks do not bump `ask.serial` or wake `SideThread`.
   - `Viewer::picture_whole` checks both `side.mode_on_screen() == Some(&self.map_mode)` and `overlay_view.matches_mode(&self.map_mode)` (unless the overlay is not coming because vision or the side refused). Live asks are issued once at the end of `map_with_state` (`ask_live`) so the side thread receives the current frame's camera, mode and overlay view.

2. **Honest limit.**
   - A future `Layer` that spawns a raw `std::thread::spawn` and writes its output solely to `gpu.scene` inside `Layer::upload` without passing a value through `Fed`, `Rebuilt`, `Whole`, `views`, `side::Products`, or `side::LayIns` is outside what `Landed<T>` catches at compile time. Closing that last surface would require wrapping `viewer_world::scene::Scene` setter inputs, which is intentionally left out to avoid coupling `viewer-world` to runtime wake tokens.

## Alternatives rejected

- **Manual `landed::bump()` calls plus a text-search CI check.** Easy to miss on error/refusal branches and enforces convention by regex rather than Rust's type system.
- **Keep polling `request_repaint_after(16 ms)` whenever anything might be pending.** Keeps the UI awake indefinitely on refused overlays or idle paused dashboards.

## Consequences

- Any async product routed into `side::Products`, `side::LayIns`, `Fed`, `Reads`, `Rebuilt`, `Whole`, `ShownVision`, `Combat`, `Overlay`, or `WorkerPool` cannot reach the paused map without advancing `LANDED` or invoking its wake callback.
- Paused dashboard maps settle to zero drawn frames and low idle CPU cycles on drawn modes, while waking promptly on seeks, mode switches, and late background answers. Refused-overlay settlement and atomic overlay answer publication follow in a stacked change.
