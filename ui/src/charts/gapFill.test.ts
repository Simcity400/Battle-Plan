import assert from 'node:assert/strict';
import test from 'node:test';
import { plotFrame } from './frame.ts';
import { curveSegments, gapRegions } from './series.ts';
import type { Run } from './series.ts';


const frame = plotFrame(600, 300, { left: 40, right: 10, top: 10, bottom: 20 }, [0, 100], [0, 400], false);



test('a gap in the recording leaves a gap in the fill', () => {
  const broken: Run[] = [{ firstHour: 0, lastHour: 20, value: 300 }, { firstHour: 30, lastHour: 60, value: 280 }];
  const lower: Run[] = [{ firstHour: 0, lastHour: 20, value: 100 }, { firstHour: 30, lastHour: 60, value: 120 }];
  const { above } = gapRegions(curveSegments(broken, 0, 100, frame.toX, frame.toY), curveSegments(lower, 0, 100, frame.toX, frame.toY), 0, 300);
  assert.equal(above.length, 2);
});
