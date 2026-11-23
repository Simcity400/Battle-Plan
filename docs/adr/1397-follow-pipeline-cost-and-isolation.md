# 1397. Follow pipeline isolation, cost balance, and work proportional to change

The follow pipeline keeps the playback clock, the time gate, the follow stream, and whole-picture installs ([ADR 0505](0505-frame-and-drawing.md)). To support 30 visible widgets at 60 fps and 1,440 game hours per second on four slowed cores, four invariants govern correctness and cost across native, the pull worker, the drawing lanes, and the page.

## Cell-group ownership across seeks and per-subscription fault isolation

A dated panel that names live cell groups owns those cell-group subscriptions across seeks. `Facts.seek` drops old facts for every subscription without releasing the panel's ownership of its cell groups, and `Facts.prune` keeps a newly owned cell group while it has no facts yet; when a panel names a cell group again, the lane clears any prior `gone` marker for that id. Native's follow stream can therefore send changed rows for a cell group after a seek without encountering a deleted base.

A decode, merge, or missing-base fault on one subscription fails only the subscription that faulted, never the whole drawing lane. `Lane.answer` and `Lane.panel` record the failure per subscription id (clearing it on a fresh whole table, reset, seek, or unfollow), and `Lane.select` emits a named failure entry only for widgets depending on that subscription or one of its live cell groups. Sibling widgets on the same lane keep serving normally; only an unrecoverable lane-level or transport-level error fails the lane.

## Panel hand-off cadence and bounded speculation

Non-exact panel rows fed through `Apart` (`crates/viewer-layer-core/src/playhead_host/views/apart.rs`) tick at 16 ms instead of 125 ms, run at normal thread priority rather than background priority, read at `playhead.head_from(head, 48ms)` while playing (falling back to `head` when paused or seeking), and skip redundant posts when `(applied, epoch)` has not moved. The `follow panels` loop in `crates/dashboard-follow/src/host.rs` polls at 8 ms while any panel is shown and stays at one frame interval when idle.

On the page, speculative targeting clamps the playing frame step to at most `1.5×` the expected hours per display frame so a single main-thread stall cannot throw the lane's target hundreds of hours ahead and latch the monotonic target high. The lane reply-delay EWMA is bounded to `[1, 4]` frames, and early entries wait in a bounded per-widget sorted queue (up to 4 entries) so later speculative windows do not displace earlier upcoming ones.

## Work proportional to change

Every stage of the pipeline scales with what changed rather than with total mounted widgets or total table rows:

- **Changed rows (`ui/src/dashboard/follow/tables.ts`)**: `patchAnswer` merges sorted changed rows by galloping/binary key lookup onto persistent 8-row chunks (`RowPatches`), copying at most `8 × columns` cells per changed row without mutating the base or retaining stale texts from overwritten rows. Worker-drawn consumers read directly from the chunked view; a flat wire `Answer` is serialized lazily via `serializeAnswer` only when a main-thread `answer:` field needs one.
- **Lane split (`ui/src/dashboard/follow/split.ts`)**: Records directed to a single lane index directly into that lane's buffer in O(1) without scanning other lanes or allocating empty buffers for idle lanes.
- **Page & Pipeline (`page.ts`, `pipeline.ts`)**: Quiet clock ticks update a shared route confirmation box for open followed widgets without scanning them, skip bounded widgets while the target remains inside `[safeLo, safeHi]`, touch only widgets with queued early entries, and notify only subscribed `dateWatchers` during `flush()`.
- **Drawing lane (`lane.ts`)**: `serve()` indexes widgets by subscription, route, dirty state, active motion, in-flight status, inspection, and next window/fact boundary (`validWindow` and `Facts.nextStart`), visiting only widgets whose inputs changed and redating confirmation-only advances (`redateEntry`) without repainting. `prune()` visits only subscriptions marked prunable since the last prune.

## Cost-based lane placement, non-starving budget packing, and no live migration

Each lane reports measured smoothed paint costs back to the pipeline on `served` messages for painted widgets. `Pipeline` tracks per-lane cost load and per-kind fallback cost, placing new and remounted widgets on the lowest-cost lane (breaking ties by widget count and keeping copies of one kind together only while within cost share). Inside `Lane.serve()`, if the next due widget's estimated cost exceeds the remaining frame budget while budget remains, the lane continues past it so cheaper due widgets pack into the remaining budget instead of starving behind one heavy widget, and any widget exceeding the frame budget advances its `served` turn proportionally (capped at 4 frames) so it cannot repeatedly sort ahead of cheaper widgets on every tick.

Live migration of an already-mounted widget between lanes is intentionally not performed. Three pieces of state are bound to the widget's lane for the lifetime of a mount: (1) subscription ids encode their lane (`id = n × lanes + lane`), (2) native allocates a panel's live cell-group subscriptions in its owner's lane class (`cells_id(lane, lanes)`), and (3) worker-local `widget.state` owns `OffscreenCanvas` handles, decoded `ImageBitmap` icon caches, and table/race layout state that cannot be moved atomically across workers without tearing down and re-registering the widget. Cost-based placement at mount, re-placement on remount, and non-starving budget packing achieve lane balance without breaking whole-picture continuity.

## Playback-aware idle renderer preloading

Idle renderer prefetching ([ADR 1203](1203-widget-async-paint.md)) pauses whenever any playback clock is playing and resumes when playback pauses. It uses `requestIdleCallback` without a forced timeout, requires `!deadline.didTimeout && deadline.timeRemaining() >= 6` before starting a chunk, and prefers inserting `<link rel="modulepreload">` tags from the Vite-emitted `meta[name="bp-widget-chunks"]` manifest so prefetching fetches bytes off the main thread instead of evaluating module graphs during playback.

[Fact store](../../ui/src/dashboard/follow/facts.ts) · [Changed rows](../../ui/src/dashboard/follow/tables.ts) · [Lane split](../../ui/src/dashboard/follow/split.ts) · [Page install](../../ui/src/dashboard/follow/page.ts) · [Pipeline](../../ui/src/dashboard/follow/pipeline.ts) · [Drawing lane](../../ui/src/draw/lane.ts) · [Apart view](../../crates/viewer-layer-core/src/playhead_host/views/apart.rs) · [Follow host](../../crates/dashboard-follow/src/host.rs) · [Renderer loading](../../ui/src/widgets/rendererLoaders.ts)
