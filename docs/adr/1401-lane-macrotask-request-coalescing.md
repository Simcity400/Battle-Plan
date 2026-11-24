# 1401. Defer post-serve drawing-lane re-schedules to a macrotask so backlogged requests coalesce

## Context

`Lane.frame()` already coalesces a pending `FrameRequest` with any newly arriving one (keeping the newest `serial`, `now`, `target`, `exact`, `seek`, `speed`, and `inspect`, while concatenating `installed` and `refused` lists). However, `Lane.run()` previously re-scheduled follow-up work from its `finally` block via `queueMicrotask()`.

In a Web Worker, incoming `postMessage` frames (`self.onmessage`) arrive as macrotasks. Because the microtask checkpoint executes immediately after a `serve()` finishes—before queued `onmessage` macrotasks are dequeued—a lane whose paint plus present takes longer than a display frame interval starts the next `serve()` on an already-stale queued request instead of coalescing newer frames waiting behind it in the worker's task queue.

Deferring every `Lane.schedule()` call—including when the lane is idle—would change idle-lane message ordering (letting a subsequent `pointer`, `frame`, or refusal/install pair run before the first frame's acknowledgements or exact render). Deferring only the post-serve re-schedule preserves exact idle-lane microtask ordering while letting a busy lane pick up frames that arrived while the previous serve was running.

## Decision

1. **Post-serve macrotask deferral (`ui/src/draw/lane.ts`, `ui/src/draw/laneDispatch.ts`, `ui/src/draw/lane.worker.ts`)**:
   - Add an optional `schedule?(run: () => void): void` callback to `LaneHost` and `LaneSide`, defaulting to `queueMicrotask` when omitted.
   - Keep `Lane.schedule()` on `queueMicrotask` so `frame()` and `wake()` on an idle lane behave identically to `main`.
   - In `Lane.run()`'s `finally`, when `this.host.schedule` is provided and either `this.request` is already queued or the completed serve took at least `this.host.budget ?? 10` ms, keep `this.working = true` until `this.host.schedule(() => this.finishRun())` runs. Any `FrameRequest`s that arrived during synchronous paint or stretch work coalesce into `this.request` before `finishRun()` releases `this.working` and schedules the next serve.
   - Export `macrotaskSchedule()` from `ui/src/draw/laneDispatch.ts` and call it from `ui/src/draw/lane.worker.ts`: when `MessageChannel` is available, it posts to a local `MessageChannel` port so a post-serve re-schedule takes whatever frames have arrived before the wake runs. When `MessageChannel` is unavailable, `macrotaskSchedule()` returns `undefined` so `Lane` keeps `queueMicrotask` unchanged.
2. **Pre-serve and post-crop superseded-seek guards in `Lane.serve()` (`ui/src/draw/lane.ts`)**:
   - Clear `this.lastExact` in `Lane.stream()` when the seek generation advances past it.
   - Check `seekSuperseded(request.seek)` at the start of `Lane.serve()` (for requests superseded while deferred after a previous serve): apply `request.installed` / `request.refused`, release freed fact bytes, and emit `host.served` with `changed: 0` without painting or calling `prune(request.now)`.
   - Check `seekSuperseded(request.seek)` again after `await this.host.crop(...)` and before `present()` / `prune(request.now)` (for requests superseded by a rewind while suspended in `crop()`): close all returned bitmaps so none leak, remove the superseded entries from `this.presented`, clear matching `widget.inflight`, restore `widget.sent` from `widget.installed`, restore unsent reset/close flags, release freed bytes, emit `host.served` with `changed: 0`, and return without calling `present()` or `prune(request.now)`.

## Consequences

- Idle and fast lanes preserve existing microtask ordering for `frame()`, `wake()`, and subsequent `pointer()` or `frame()` messages.
- When a lane's serve is backlogged or exceeds its budget (`10 ms`), the next serve takes whatever frames have arrived before the local wake rather than stepping strictly through older queued requests.
- Observed on `--profile weak8` at 1,440 h/s across three alternating pairs against `main` (all 6 exact audits passed in every run): `game-score` lag p95 dropped from 4,906 h to 96 h (Pair 1: main baseline usable=false, branch usable=false), 4,871 h to 270 h (Pair 2: main usable=true, branch usable=false), and 5,479 h to 97 h (Pair 3: main usable=true, branch usable=false); `occupied-states` dropped in all three pairs (45→23 h, 111→23 h, 203→52 h). A remaining hypothesis for occasional higher-lag bursts under heavier background contention is that end-to-end acceptance still depends on `Page`'s speculative lead keeping `[lo, hi]` valid through presentation arrival; tuning page pacing is left to a separate follow-up.
- A superseded seek neither leaks cropped bitmaps nor prunes facts belonging to the newer seek generation.
- No changes to `Pipeline`, `Page`, bitmap cropping, `stretch()` ordering, or `ui/src/families/`.
