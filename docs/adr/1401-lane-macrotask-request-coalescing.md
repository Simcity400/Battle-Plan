# 1401. Lane inbox separation and post-serve macrotask deferral

## Context

`Lane.frame()` previously merged every pending `FrameRequest` into a single `this.request` field (concatenating `installed` and `refused` across frames and overwriting an `exact` request with a later speculative one), and `Lane.run()` re-scheduled follow-up work from `finally` via `queueMicrotask()`.

In a Web Worker, incoming `postMessage` frames (`self.onmessage`) arrive as macrotasks. Because the microtask checkpoint executes immediately after a `serve()` finishes—before queued `onmessage` macrotasks are dequeued—a lane whose paint plus present (including weak-CPU `stretch()`) takes longer than a display frame interval starts the next `serve()` on an already-stale queued request instead of coalescing newer frames waiting behind it in the worker's task queue.

At the same time, deferring idle frames or merging acknowledgements and exact requests across frames during a serve or post-serve hand-over changes user-visible ordering: a pointer after an acknowledgement can hit the previous picture, a `kept content` refusal followed by a valid install can clear the install (#306), and an exact request followed by a speculative request can drop the exact render (#306).

## Decision

1. **Three-part lane inbox (`ui/src/draw/lane.ts`)**:
   - **Acknowledgements (FIFO of per-frame batches)**: never merged across frames. When no serve is executing (`!this.serving`, including idle and post-serve hand-over), each frame's `installed` then `refused` batch applies immediately on receipt and releases freed bytes so `pointer()` always sees every acknowledgement that arrived outside a serve. While a serve is executing (`this.serving === true`, including across `await host.crop()`), batches queue in arrival order and flush in FIFO order when that serve finishes, before any hand-over wake.
   - **Exact request (slot of one)**: holds the newest `exact: true` frame, served ahead of any speculative frame and never overwritten by a speculative frame (a newer `exact: true` replaces an older exact and clears any older speculative request; synthetic `wake()` replays of `lastExact` remain replaceable when playback resumes with a speculative frame).
   - **Speculative request (slot of one)**: holds the newest `exact: false` frame.
2. **Post-serve macrotask deferral (`ui/src/draw/lane.ts`, `ui/src/draw/laneDispatch.ts`, `ui/src/draw/lane.worker.ts`)**:
   - Add an optional `schedule?(run: () => void): void` callback to `LaneHost` and `LaneSide`, defaulting to `queueMicrotask` when omitted.
   - Keep `Lane.schedule()` on `queueMicrotask` so idle and fast lanes behave identically to `main`.
   - In `Lane.run()`'s `finally`, after flushing queued acknowledgement batches, when `this.host.schedule` is provided and either a request is queued or the completed serve took at least `this.host.budget ?? 10` ms, arm any already-due motion callback immediately if no request is queued and keep `this.working = true` until `this.host.schedule(() => this.finishRun())` runs. Frames delivered to the worker before the wake are merged into the inbox before the next paint begins.
   - Export `macrotaskSchedule()` and `cropAll()` from `ui/src/draw/laneDispatch.ts` and wire them in `ui/src/draw/lane.worker.ts` (`startLaneWorker`): when `MessageChannel` is available, `macrotaskSchedule()` posts to a local `MessageChannel` port; when unavailable, it returns `undefined` so `Lane` keeps `queueMicrotask`. `cropAll()` closes any fulfilled sibling bitmaps before rethrowing if a crop batch partially rejects.
3. **Pre-serve and post-crop superseded-seek guards and cancelled-paint reporting (`ui/src/draw/lane.ts`, `ui/src/draw/laneMessages.ts`, `ui/src/draw/notifications.ts`, `ui/src/dashboard/follow/pipeline.ts`, `tools/e2e/datedPlayback.mjs`)**:
   - Clear older `lastExact` in `Lane.stream()`.
   - Check `seekSuperseded(request.seek)` at the start of `Lane.serve()`: release freed fact bytes, emit `host.served` with `changed: 0, cancelled: true`, and skip paint and `prune(request.now)`.
   - Check `seekSuperseded(request.seek)` again after `await this.host.crop(...)` and before `present()` / `prune(request.now)`: close all returned bitmaps, retire `entries` from `this.presented`, clear matching `widget.inflight`, clear `widget.sent = {}` for affected widgets (so the first valid presentation on the new seek sends full content instead of `SAME` against content the page cleared), restore unsent reset/close flags, release freed bytes, emit `host.served` with `changed: 0, cancelled: true`, and return without calling `present()` or `prune(request.now)`.
   - Forward `cancelled: true` onto `workerFrame` records and exclude cancelled frames from `emptyMs` and empty-page baseline `workerMeanMs` in `tools/e2e/datedPlayback.mjs`.

## Consequences

- Idle lanes preserve existing microtask ordering; both hand-over and mid-serve windows preserve per-frame acknowledgement order and exact-before-speculative delivery (fixing #306).
- Frames delivered to the worker before the post-serve wake merge in the inbox before the next paint starts, keeping slow paints close to the newest request.
- Observed on `--profile weak8` at 1,440 h/s across three alternating pairs against `main`, each started only after system CPU was below 30% (all 6 exact audits passed in every run): `game-score` lag p95 dropped from 5,070 h to 87 h (Pair 1: main baseline `usable=true`, branch `usable=true`), 3,477 h to 98 h (Pair 2: main `usable=true`, branch `usable=true`), and 4,552 h to 111 h (Pair 3: main `usable=false`, branch `usable=true`), while the other five measured widgets (`scoreboard`, `occupied-states`, `division-template`, `campaign-clock`, `faction-strength`) stayed within run-to-run noise.
- Superseded serves and partially failed crop batches close their bitmaps without leaking, never prune facts from a newer seek generation, and do not pollute `emptyMs`.
- No changes to `Page`, bitmap cropping internals, `stretch()` ordering, or `ui/src/families/`.
