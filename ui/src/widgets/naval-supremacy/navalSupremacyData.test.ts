import test from 'node:test';
import assert from 'node:assert/strict';
import { STEP, controlOf, followedEnd, mergeSupremacy, runOn, supremacyDays, supremacyRows, unknownReason, validateSupremacy } from './navalSupremacyData.ts';

const reply = validateSupremacy({
  firstHour: 30, lastHour: 100, firstDayHour: 24, dayHours: 24, days: 4,
  unknown: [{ firstDay: 2, lastDay: 2, reason: 'Not in this recording' }],
  regions: [{ id: '5', name: 'North Sea', runs: [{ firstDay: 0, lastDay: 1, control: 1, friendly: '300000', enemy: '0', share: '100000', own: '300000' },
    { firstDay: 3, lastDay: 3, control: null, friendly: null, enemy: null, share: null, own: '0' }] }],
  scale: 100000, warnings: [],
});
const colours = { friendly: 'f', enemy: 'e', neither: 'n' } as const;
const labels = { friendly: 'Friendly control', enemy: 'Enemy control', neither: 'Contested' } as const;

test('days are cut to the window and the reply; a day not known has no read', () => {
  const days = supremacyDays(reply, [0, 200]);
  assert.deepEqual(days.map(day => [day.index, day.firstHour, day.lastHour, day.readHour]), [[0, 30, 47, 47], [1, 48, 71, 71], [2, 72, 95, null], [3, 96, 100, 100]]);
  assert.deepEqual(supremacyDays(reply, [60, 80]).map(day => day.index), [1, 2]);
  assert.equal(unknownReason(reply, 2), 'Not in this recording');
});

test('a known day without an admitted control is hatched, an unknown day blank, never contested', () => {
  const [row] = supremacyRows(reply.regions, supremacyDays(reply, [0, 200]), colours, labels);
  assert.deepEqual(row.cells.map(cell => cell.unknown ? 'hatched' : cell.parts[0]?.id ?? 'blank'), ['friendly', 'friendly', 'blank', 'hatched']);
  assert.equal(controlOf(runOn(reply.regions[0].runs, 2)), null);
});

test('a run past the reply’s days is refused', () => {
  assert.throws(() => validateSupremacy({ ...reply, regions: [{ id: '5', name: 'x', runs: [{ firstDay: 3, lastDay: 4, control: 0, friendly: null, enemy: null, share: null, own: null }] }] }));
});

test('paused or after a seek the view ends at the head, never at the end of its month ahead', () => {
  const head = 43_800_000 + 1936 * 8760 + 11;
  assert.equal(followedEnd(head, false), head);
  assert.equal(followedEnd(head + 0.5, false), head);
  const end = followedEnd(head, true);
  assert.ok(end >= head && end < head + STEP && (end + 1) % STEP === 0, 'while playing it moves by whole months');
});

test('the windows held join into one reply of their days, a run or unknown span across their edge one', () => {
  const run = (firstDay: number, lastDay: number, control: number | null) => ({ firstDay, lastDay, control, friendly: null, enemy: null, share: null, own: null });
  const window = (firstDayHour: number, days: number, runs: ReturnType<typeof run>[], unknown: { firstDay: number; lastDay: number; reason: string }[]) => validateSupremacy({
    firstHour: firstDayHour, lastHour: firstDayHour + days * 24 - 1, firstDayHour, dayHours: 24, days, unknown,
    regions: [{ id: '5', name: 'North Sea', runs }], scale: 100000, warnings: [`w${firstDayHour}`] });
  const merged = mergeSupremacy(new Map([
    [1, window(240, 10, [run(0, 3, 1), run(4, 9, 0)], [])],
    [0, window(0, 10, [run(5, 9, 0)], [{ firstDay: 0, lastDay: 4, reason: 'Not in this recording' }])],
  ]));
  assert.ok(merged);
  assert.deepEqual([merged.firstHour, merged.lastHour, merged.days], [0, 479, 20]);
  assert.deepEqual(merged.regions[0].runs.map(item => [item.firstDay, item.lastDay, item.control]), [[5, 9, 0], [10, 13, 1], [14, 19, 0]]);
  assert.deepEqual(merged.unknown, [{ firstDay: 0, lastDay: 4, reason: 'Not in this recording' }]);
  assert.deepEqual(merged.warnings, ['w240', 'w0']);
  const joined = mergeSupremacy(new Map([[0, window(0, 2, [run(0, 1, 1)], [])], [1, window(48, 2, [run(0, 1, 1)], [])]]));
  assert.deepEqual(joined?.regions[0].runs.map(item => [item.firstDay, item.lastDay]), [[0, 3]], 'equal days across the edge are one run');
  assert.equal(mergeSupremacy(new Map()), null);
});
