import assert from 'node:assert/strict';
import type { HeatLaneRun } from './lanesModel.ts';

export function assertRunCounts(start: number, at: (hour: number) => readonly HeatLaneRun[]): void {
  const arrays = new Set<readonly HeatLaneRun[]>();
  const runs = new Set<HeatLaneRun>();
  const cells = new Set<HeatLaneRun['cell']>();
  const initial = at(start);
  for (let hour = start; hour < start + 100; hour++) {
    const first = at(hour);
    const second = at(hour);
    for (const row of [first, second]) {
      arrays.add(row);
      for (const run of row) {
        runs.add(run);
        cells.add(run.cell);
      }
    }
    assert.equal(first, second);
    assert.equal(first.at(-1)!.lastHour, hour);
    assert.equal(first.length, initial.length);
    for (let i = 0; i < initial.length - 1; i++) assert.equal(first[i], initial[i]);
  }
  assert.equal(initial.at(-1)!.lastHour, start);
  assert.deepEqual([arrays.size, runs.size, cells.size], [100, initial.length + 99, new Set(initial.map(run => run.cell)).size]);
}
