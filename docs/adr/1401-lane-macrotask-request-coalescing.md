# 1401. Lane acknowledgement FIFO and speculative post-serve hand-over

## Context

`Lane.frame()` previously merged every pending `FrameRequest` into a single `this.request` field (concatenating `installed` and `refused` across frames) and `Lane.run()` re-scheduled follow-up work from `finally` via `queueMicrotask()`.

In a Web Worker, incoming `postMessage` frames (`self.onmessage`) arrive as macrotasks. Because the microtask checkpoint executes immediately after a `serve()` finishes—before queued `onmessage` macrotasks are dequeued—a lane whose paint plus present (including weak-CPU `stretch()`) takes longer than a display frame interval starts the next `serve()` on an already-stale queued request instead of coalescing newer frames waiting behind it in the worker's task queue.

At the same time, deferring idle or exact frames, or merging acknowledgements across frames during a serve or post-serve hand-over, changes user-visible ordering: a pointer after an acknowledgement can hit the previous picture, a `kept content` refusal followed by a valid install can clear the install (#306), and an exact frame held behind a speculative hand-over can be lost.

## Decision

1. **Acknowledgement FIFO and single request slot (`ui/src/draw/lane.ts`)**:
   - **Acknowledgements (FIFO of per-frame batches)**: never merged across frames. When no serve is executing (`!this.serving`, including idle and post-serve hand-over), each frame's `installed` then `refused` batch applies immediately on receipt and releases freed bytes so `pointer()` always sees every acknowledgement that arrived outside a serve. While a serve is executing (`this.serving === true`, including across `await host.crop()`), batches queue in arrival order and flush in FIFO order when that serve finishes, before any hand-over wake. Processing time spent applying acknowledgements outside `serve()` is charged once to the next serve's paint budget, `stretch()`, and `served.ms`.
   - **Single pending request slot**: preserves `main`'s single `this.request` and `this.lastExact` replacement rules (exact frames are never kept in a separate mid-serve slot whose unacknowledged `SAME` could break a following speculative frame).
2. **Speculative-only post-serve hand-over (`ui/src/draw/lane.ts`, `ui/src/draw/laneDispatch.ts`, `ui/src/draw/lane.worker.ts`)**:
   - Add an optional `schedule?(run: () => void): void` callback to `LaneHost` and `LaneSide`, defaulting to `queueMicrotask` when omitted.
   - Keep `Lane.schedule()` on `queueMicrotask` so idle and fast lanes behave identically to `main`.
   - Exact frames are never held in a hand-over: at serve completion, if `this.request?.exact` is true, start it immediately from a microtask without a wake; if an `exact: true` frame arrives during a hand-over, end the hand-over at once, start it from a microtask, and invalidate the late wake.
   - In `Lane.run()`'s `finally`, after flushing queued acknowledgement batches, only when `this.host.schedule` is provided, `!this.request?.exact`, and either a speculative request is queued or the completed serve took at least `this.host.budget ?? 10` ms, arm any already-due motion callback immediately if no request is queued and wait for `this.host.schedule(...)`. Speculative frames delivered to the worker before the wake collapse into the newest before the next paint begins.
   - Export `macrotaskSchedule()` and `cropAll()` from `ui/src/draw/laneDispatch.ts` and wire them in `ui/src/draw/lane.worker.ts` (`startLaneWorker`): when `MessageChannel` is available, `macrotaskSchedule()` posts to a local `MessageChannel` port; when unavailable, it returns `undefined` so `Lane` keeps `queueMicrotask`. `cropAll()` closes any fulfilled sibling bitmaps before rethrowing if a crop batch partially rejects.
3. **Pre-serve and post-crop superseded-seek guards and cancelled-paint reporting (`ui/src/draw/lane.ts`, `ui/src/draw/laneMessages.ts`, `ui/src/draw/notifications.ts`, `ui/src/dashboard/follow/pipeline.ts`, `tools/e2e/datedPlayback.mjs`)**:
   - Clear older `lastExact` in `Lane.stream()`.
   - Check `seekSuperseded(request.seek)` at the start of `Lane.serve()`: release freed fact bytes, emit `host.served` with `changed: 0, cancelled: true`, and skip paint and `prune(request.now)`.
   - Check `seekSuperseded(request.seek)` again after `await this.host.crop(...)` and before `present()` / `prune(request.now)`: close all returned bitmaps, retire `entries` from `this.presented`, clear matching `widget.inflight`, mark affected widgets `forceFull = true` (consumed only when a full picture entry is built, immune to intervening refusals so the first valid presentation on the new seek never sends `SAME` against content the page cleared), restore unsent reset/close flags, release freed bytes, emit `host.served` with `changed: 0, cancelled: true`, and return without calling `present()` or `prune(request.now)`.
   - Forward `cancelled: true` onto `workerFrame` records and exclude cancelled frames from worker presentation rate, gaps, `emptyMs`, and empty-page baseline `workerMeanMs` in `tools/e2e/datedPlayback.mjs`.

## Consequences

- Idle lanes and exact frames preserve `main`'s microtask ordering; cross-frame acknowledgement ordering is fixed both mid-serve and during hand-over (partially addressing #306).
- Speculative frames delivered to the worker before a post-serve wake collapse into the newest before the next paint starts, keeping slow paints close to the playhead.
- Observed on `--profile weak8` at 1,440 h/s across three alternating pairs against `main`, each started only after system CPU was below 30% (all 6 exact audits passed in every run): `game-score` lag p95 dropped from 5,070 h to 87 h (Pair 1: main baseline `usable=true`, branch `usable=true`), 3,477 h to 98 h (Pair 2: main `usable=true`, branch `usable=true`), and 4,552 h to 111 h (Pair 3: main `usable=false`, branch `usable=true`), while the other five measured widgets (`scoreboard`, `occupied-states`, `division-template`, `campaign-clock`, `faction-strength`) stayed within run-to-run noise. Confirmed on the final Round 4 revision with an alternating `weak8` pair (`game-score,scoreboard`, pre-pair CPU 17.8%, both baselines `usable=true`, both exact audits passing): `game-score` lag p95 dropped from 3,492 h to 365 h (p50 1,157 h to 54 h) even as host CPU rose to 50.3% during the branch run (`worker p95` 51.3 ms vs `main` 39.5 ms).
- Superseded serves and partially failed crop batches close their bitmaps without leaking, never prune facts from a newer seek generation, and do not count as presented or idle frames in the audit.
- No changes to `Page`, bitmap cropping internals, `stretch()` ordering, or `ui/src/families/`.
