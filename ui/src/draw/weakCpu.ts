/** Spins between clock reads: enough that the clock is read about once every few microseconds, not every iteration. */
const SPIN = 4096;
let sink = 0;

/** The weak-PC tier inside a worker, which Chromium's CPU throttling never reaches (it throttles only a page's main
 * thread): after `work` ms of a frame's drawing, the thread stays busy `(rate - 1) x work` ms more, so the worker's
 * frames take what they would on a CPU `rate` times slower. Rate 1 costs nothing. `now` is the clock as the frame last
 * read it. Returns the clock when it ends. */
export function stretch(work: number, rate: number, now: number): number {
  if (!(rate > 1) || !(work > 0)) return now;
  const until = now + work * (rate - 1);
  let at = now;
  while (at < until) {
    for (let index = 0; index < SPIN; index++) sink = (sink + index) | 0;
    at = performance.now();
  }
  return at;
}
