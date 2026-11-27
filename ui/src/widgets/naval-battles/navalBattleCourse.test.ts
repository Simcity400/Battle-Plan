import test from 'node:test';
import assert from 'node:assert/strict';
import { navalCourseFigures, navalCourseGaps, NOT_AT_THIS_HOUR, validateNavalBattleCourse } from './navalBattleCourse.ts';
import type { NavalCourseRead } from '../../generated/dashboard';

const HOUR = 43_800_000 + 1940 * 8760;
const read = (hour: number, extra: Partial<NavalCourseRead> = {}): NavalCourseRead => ({ hour, result: 'ongoing', ships: [3, 2], convoys: [0, 4],
  shipsSunk: [0, 1], convoysSunk: [0, 2], icLost: [0, 12.5], resulted: false, ...extra });

test('the sides band reads each side at the hour shown, with losses marked as counted so far while the battle is live', () => {
  const course = validateNavalBattleCourse({ branchId: '1', warnings: [], missing: [{ firstHour: HOUR + 3, lastHour: HOUR + 5, reason: 'Naval battle census incomplete' }],
    reads: [read(HOUR), read(HOUR + 2, { ships: [3, null], icLost: [0, null] }), read(HOUR + 6, { result: 'unknown' }), read(HOUR + 8, { result: 'defenderWon', shipsSunk: [2, 1], resulted: true })] });
  const [before] = navalCourseFigures(course, HOUR - 1);
  assert.deepEqual(before.ships, { missing: NOT_AT_THIS_HOUR });
  const [attacker, defender] = navalCourseFigures(course, HOUR + 1);
  assert.deepEqual([attacker.ships, defender.convoys, defender.sunk, defender.icLost],
    [{ value: 3, soFar: false }, { value: 4, soFar: false }, { value: 3, soFar: true }, { value: 12.5, soFar: true }]);
  const [, unread] = navalCourseFigures(course, HOUR + 2);
  assert.ok('missing' in unread.ships && 'missing' in unread.icLost, 'a side figure the recording does not hold is missing, never zero');
  const [inGap] = navalCourseFigures(course, HOUR + 4);
  assert.deepEqual(inGap.ships, { missing: 'Naval battle census incomplete' }, 'an hour the census did not read carries no earlier reading');
  const [gone] = navalCourseFigures(course, HOUR + 7);
  assert.deepEqual(gone.sunk, { value: 0, soFar: true }, 'a battle gone without a recorded result keeps its losses as a lower bound');
  const [settled] = navalCourseFigures(course, HOUR + 9);
  assert.deepEqual(settled.sunk, { value: 2, soFar: false }, 'after the result the losses are final');
  assert.deepEqual(navalCourseGaps(course, HOUR, HOUR + 4), [{ first: HOUR + 3, last: HOUR + 4 }]);
  assert.deepEqual(navalCourseGaps(course, HOUR + 6, HOUR + 9), []);
});

test('a malformed course is refused', () => {
  assert.throws(() => validateNavalBattleCourse({ branchId: '1', warnings: [], missing: [], reads: [read(HOUR + 1), read(HOUR)] }));
  assert.throws(() => validateNavalBattleCourse({ branchId: '1', warnings: [], missing: [], reads: [read(HOUR, { ships: [-1, 0] })] }));
  assert.throws(() => validateNavalBattleCourse({ branchId: '1', warnings: [], missing: [], reads: [read(HOUR, { result: 'won' as never })] }));
});
