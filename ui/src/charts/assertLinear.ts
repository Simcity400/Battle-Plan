/** For node tests: that work grows about linearly with its size. `make(count)` builds
 * the work for `count` items and returns a function that runs it; the best of nine
 * timed runs (after three warm-ups) for four times the items must take less than
 * eight times as long, which a quadratic cost (sixteen times) cannot. */
import assert from 'node:assert/strict';

export function assertLinear(make: (count: number) => () => unknown, what = 'items', small = 5_000): void {
  const best = (run: () => unknown) => {
    for (let round = 0; round < 3; round++) run();
    let fastest = Infinity;
    for (let round = 0; round < 9; round++) {
      const started = performance.now();
      run();
      fastest = Math.min(fastest, performance.now() - started);
    }
    return fastest;
  };
  const few = best(make(small));
  const many = best(make(small * 4));
  assert.ok(many < few * 8, `${small.toLocaleString('en-US')} ${what} ${few.toFixed(2)} ms, ${(small * 4).toLocaleString('en-US')} ${what} ${many.toFixed(2)} ms`);
}
