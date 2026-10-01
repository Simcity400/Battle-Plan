import assert from 'node:assert/strict';

/** Test helper: asserts a run takes about linear time in its size, whatever the
 * machine's load. Each of fifteen pairs times one run at 20,000 against four runs at
 * 5,000 back to back, so both halves last about as long and see the same load; the
 * median of the pairs' ratios is about 1 for linear work (a little more with a log)
 * and 4 for a quadratic pass. A load spike skews single pairs, not the median. */
export function assertLinear(what: string, make: (count: number) => () => unknown): void {
  const small = make(5_000);
  const large = make(20_000);
  for (let round = 0; round < 3; round++) { small(); large(); }
  const ratios: number[] = [];
  for (let pair = 0; pair < 15; pair++) {
    let started = performance.now();
    for (let run = 0; run < 4; run++) small();
    const four = performance.now() - started;
    started = performance.now();
    large();
    ratios.push((performance.now() - started) / Math.max(four, 1e-3));
  }
  const median = ratios.sort((a, b) => a - b)[7];
  assert.ok(median < 2.5, `20,000 ${what} took ${median.toFixed(2)} times four runs of 5,000`);
}
