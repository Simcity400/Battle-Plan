import assert from 'node:assert/strict';
import test from 'node:test';
import type { AirTimeline } from './airSuperiority';
import type { BoundTrack } from './model';
import { airColourPlan } from './useAirSuperiorityColours.ts';

const branch = { id: '1', sessionId: '1', firstHour: 0, lastHour: 47, parentId: null, parentHour: null, sampleBoundary: 'Synthetic', gaps: [] } as BoundTrack['branch'];
const side = (key: string, tag: string) => ({ key, tag, faction: null, colourFaction: null, name: tag, leader: null, coBelligerents: 1 });
const reply: AirTimeline = { view: 'timeline', branchId: '1', available: true, firstHour: 0, lastHour: 47, days: [23, 47],
  sides: [side('w:1:101', 'Y'), side('w:2:102', 'X')],
  anchorReads: { 'w:1:101': ['X', 10], 'w:2:102': ['X', 30] },
  segments: { X: [{ firstHour: 0, lastHour: 23 }, { firstHour: 24, lastHour: 47 }], Y: [{ firstHour: 24, lastHour: 47 }] },
  rows: [], excluded: [], theatreReads: [], scale: 100000, dayHours: 24, badBelow: 40000, goodAbove: 60000, warnings: [] };

test('native coalition anchors preserve independent dates when another holder reuses the tag', () => {
  const plan = airColourPlan(reply, undefined, 47, null, branch);
  assert.deepEqual(plan.wanted, [{ tag: 'X', hour: 10 }, { tag: 'X', hour: 30 }]);
  assert.deepEqual(plan.sources.get(JSON.stringify(['w:1:101', 47])), { tag: 'X', hour: 10 });
  assert.deepEqual(plan.sources.get(JSON.stringify(['w:2:102', 47])), { tag: 'X', hour: 30 });
  const missing = airColourPlan({ ...reply, segments: { X: [{ firstHour: 24, lastHour: 47 }] } }, undefined, 47, null, branch);
  assert.equal(missing.sources.has(JSON.stringify(['w:1:101', 47])), false);
});

test('no-majority faction colours use the established country read and stay unknown without it', () => {
  const faction = { ...side('f:7', 'Y'), faction: '7', leader: 'Y' };
  const known = airColourPlan({ ...reply, sides: [faction], anchorReads: { 'f:7': ['X', 10] } }, undefined, 47, null, branch);
  assert.deepEqual(known.wanted, [{ tag: 'X', hour: 10 }]);
  const unknown = airColourPlan({ ...reply, sides: [faction], anchorReads: {} }, undefined, 47, null, branch);
  assert.deepEqual(unknown.wanted, []);
});
