const SPIN = 4096;
let sink = 0;

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
