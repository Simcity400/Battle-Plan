import assert from 'node:assert/strict';
import test from 'node:test';
import type { AirTimeline } from './airSuperiority';
import type { BoundTrack } from './model';
import { nameKey } from './countryFacts.ts';
import { airColourGroups, airColourPlan } from './useAirSuperiorityColours.ts';
import type { AirColourSources } from './useAirSuperiorityColours';

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
  assert.equal(plan.sources.get(47)?.sides.get('w:1:101'), nameKey(10, 'X'));
  assert.equal(plan.sources.get(47)?.sides.get('w:2:102'), nameKey(30, 'X'));
  const missing = airColourPlan({ ...reply, segments: { X: [{ firstHour: 24, lastHour: 47 }] } }, undefined, 47, null, branch);
  assert.equal(missing.sources.get(47)?.sides.has('w:1:101'), false);
});

test('sampled country colours use supplying reads rather than represented day endpoints', () => {
  const sampled: AirTimeline = { ...reply, sides: [side('c:X', 'X')], anchorReads: {}, sampleHours: [10, 10] };
  const plan = airColourPlan(sampled, undefined, 10, null, branch);
  assert.deepEqual([...plan.sources.keys()], [10]);
  assert.deepEqual(plan.wanted, [{ tag: 'X', hour: 10 }]);
});

test('no-majority faction colours use the established country read and stay unknown without it', () => {
  const faction = { ...side('f:7', 'Y'), faction: '7', leader: 'Y' };
  const known = airColourPlan({ ...reply, sides: [faction], anchorReads: { 'f:7': ['X', 10] } }, undefined, 47, null, branch);
  assert.deepEqual(known.wanted, [{ tag: 'X', hour: 10 }]);
  const unknown = airColourPlan({ ...reply, sides: [faction], anchorReads: {} }, undefined, 47, null, branch);
  assert.deepEqual(unknown.wanted, []);
});

test('colour groups are coloured again only where reads arrive, and alike hours share one group', () => {
  const plan = (entries: [number, [string, string][]][]) => new Map<number, AirColourSources>(entries.map(([at, sides]) => [at, { sides: new Map(sides) }]));
  const history = plan([[23, [['c:X', nameKey(23, 'X')]]], [47, [['c:X', nameKey(47, 'X')]]], [71, [['c:X', nameKey(71, 'X')]]]]);
  const style = (tag: string, color: [number, number, number]) => ({ tag, name: tag, color });
  const styles = new Map<string, unknown>([[nameKey(23, 'X'), style('X', [16, 32, 48])], [nameKey(47, 'X'), style('X', [16, 32, 48])], [nameKey(71, 'X'), style('X', [112, 128, 144])]]);
  const first = airColourGroups(history, new Map(), key => styles.get(key), '#000000', false);
  assert.equal(first.get(23)?.group, first.get(47)?.group);
  assert.equal(first.get(71)?.group.sides.get('c:X'), '#708090');
  // The current day's read at the history's last hour joins it there; other hours keep their groups.
  styles.set(nameKey(5, 'Y'), style('Y', [1, 2, 3]));
  const next = airColourGroups(history, plan([[71, [['w:1', nameKey(5, 'Y')]]]]), key => styles.get(key), '#000000', false, first);
  assert.equal(next.get(23), first.get(23));
  assert.equal(next.get(47), first.get(47));
  assert.deepEqual([...next.get(71)!.group.sides], [['c:X', '#708090'], ['w:1', '#010203']]);
});
