# 1401. Defer drawing-lane serves to a macrotask so backlogged requests coalesce

## Context

`Lane.frame()` already coalesces a pending `FrameRequest` with any newly arriving one (keeping the newest `serial`, `now`, `target`, `exact`, `seek`, `speed`, and `inspect`, while concatenating `installed` and `refused` lists). However, `Lane.schedule()` previously dispatched `this.run()` via `queueMicrotask()`.

In a Web Worker, every incoming `postMessage` (`self.onmessage` for page frames and `port.onmessage` for pushed follow/panel packets) is a macrotask. Because the microtask checkpoint executes immediately after each macrotask before the next queued `onmessage` macrotask is dequeued, a lane whose `serve()` exceeded one display frame interval ran a full `serve()` for every backlogged `FrameRequest` in strict FIFO order without ever draining newer `frame` messages sitting behind it in the worker's task queue.

## Decision

1. **Optional scheduler on `LaneHost` / `LaneSide` (`ui/src/draw/lane.ts`, `ui/src/draw/laneDispatch.ts`, `ui/src/draw/lane.worker.ts`)**:
   - Add an optional `schedule?(run: () => void): void` callback to `LaneHost` and `LaneSide`, defaulting to `queueMicrotask` when omitted.
   - In `ui/src/draw/lane.worker.ts`, when `MessageChannel` is available, schedule `Lane.run()` by posting to a local `MessageChannel` port so the run executes as a macrotask after any already-queued `onmessage` frames and follow/panel packets have been drained and coalesced into `this.request` and `this.facts`. When `MessageChannel` is unavailable, omit `schedule` so `Lane` keeps `queueMicrotask` unchanged.
2. **Superseded-seek guard in `Lane.serve()` (`ui/src/draw/lane.ts`)**:
   - Because deferring `run()` to a macrotask allows a `stream` seek switch and next-seek packets to execute between `schedule()` and `run()`, `Lane.serve()` checks whether any bound route's seek generation has advanced past `request.seek` (and `Lane.stream()` clears any older `lastExact`).
   - When superseded, `Lane.serve()` still applies `request.installed` / `request.refused`, releases freed fact bytes, and emits `host.served` with `changed: 0`, without painting or calling `prune(request.now)` against the newer seek generation.

## Consequences

- When a lane's paint exceeds the frame interval, queued `FrameRequest`s coalesce to the newest target before the next `serve()` begins rather than draining one stale frame per microtask.
- Measured operating range (`--profile weak8`, 1,440 h/s, three alternating pairs against `main`): in the quietest pair (`game-score` worker p95 ~40 ms), `game-score` lag p95 dropped from 3,455 h to 98 h (and 5,564 h to 104 h in Pair 2), while under severe background contention (Pair 1: 47–56% OS CPU with a concurrent app, worker p95 82 ms exceeding `Page`'s `MAX_AHEAD = 4` lead horizon) lag still rose to 2,695 h because end-to-end acceptance still depends on `Page`'s speculative lead keeping `[lo, hi]` valid through presentation arrival; tuning page pacing is left to a separate follow-up.
- No changes to `Pipeline`, `Page`, bitmap cropping, `stretch()` ordering, or `ui/src/families/`.
