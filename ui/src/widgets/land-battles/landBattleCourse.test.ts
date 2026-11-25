import assert from 'node:assert/strict';
import test from 'node:test';
import { courseDivisions, courseEquipment, courseExtent, courseManpower, validateLandBattleCourse, validateLandBattleTerrain } from './landBattleCourse.ts';

const run = (firstHour: number, lastHour: number, counts: [number, number], rosterUnknown = false) =>
  ({ firstHour, lastHour, counts, rosterUnknown, countries: [['A'], ['B']] as [string[], string[]], countriesUnknown: [false, false] as [boolean, boolean], duration: 0 });
const wire = (losses: unknown[], runs = [run(10, 20, [3, 2])], designs = [{ name: 'Rifle', variant: '7' }, { name: 'Rifle', variant: '8' }, { name: null, variant: '9' }]) => ({
  battle: { id: 'b', provinceId: 1, start: { hour: 10, bound: 'exact' }, end: null, endUnknownHour: null, counterBase: 0, runs, losses, result: null },
  designs, contentWarnings: [] });

test('a course rebuilds every read by design and refuses orphan changes and repeated designs', () => {
  const course = validateLandBattleCourse(wire([
    { hour: 10, terminal: false, sides: [{ manpower: 5, variants: [[0, '100000'], [1, '50000']] }, { manpower: 'Not in this recording', unknown: 'Not in this recording' }] },
    { hour: 12, terminal: false, sides: [{ manpower: 9, changed: [[1, '250000']] }, { manpower: 2, variants: [[2, '-100000']] }] },
    { hour: 13, terminal: false, sides: 'Hourly losses not recorded' },
    { hour: 15, terminal: true, sides: [{ manpower: 11, variants: [[1, '300000'], [0, '0']] }, { manpower: 4, variants: [[2, '-100000']] }] },
  ]));
  assert.equal(course.reads.length, 4);
  const second = course.reads[1].sides;
  assert.ok(typeof second !== 'string');
  assert.deepEqual([...(second[0].equipment as Map<number, bigint>)], [[0, 100000n], [1, 250000n]]);
  for (const losses of [
    [{ hour: 10, terminal: false, sides: [{ manpower: 1, changed: [] }, { manpower: 1, unknown: 'x' }] }],
    [{ hour: 10, terminal: false, sides: [{ manpower: 1, variants: [[0, '1'], [0, '2']] }, { manpower: 1, unknown: 'x' }] }],
    [{ hour: 10, terminal: false, sides: [{ manpower: 1, variants: [[3, '1']] }, { manpower: 1, unknown: 'x' }] }],
    [{ hour: 10, terminal: true, sides: 'x' }, { hour: 11, terminal: false, sides: 'x' }],
  ]) assert.throws(() => validateLandBattleCourse(wire(losses)), /malformed/);
});

test('equipment by design sums same names exactly, leaves zero out and never reads past the shown hour', () => {
  const course = validateLandBattleCourse(wire([
    { hour: 10, terminal: false, sides: [{ manpower: 5, variants: [[0, '100000'], [1, '50000'], [2, '0']] }, { manpower: 1, variants: [] }] },
    { hour: 15, terminal: true, sides: [{ manpower: 11, variants: [[0, '100000'], [1, '350001'], [2, '0']] }, { manpower: 4, variants: [[2, '-100000']] }] },
  ]));
  const early = courseEquipment(course, 14)!;
  assert.equal(early.hour, 10);
  assert.equal(early.terminal, false);
  assert.deepEqual(early.sides[0], [{ name: 'Rifle', raw: 150000n, designs: 2, variants: ['7', '8'] }]);
  assert.deepEqual(early.zero, [1, 0]);
  assert.deepEqual(early.sides[1], []);
  const final = courseEquipment(course, 40)!;
  assert.equal(final.terminal, true);
  assert.deepEqual((final.sides[0] as { raw: bigint }[]).map(entry => entry.raw), [450001n]);
  assert.deepEqual(final.sides[1], [{ name: 'Name not recorded', raw: -100000n, designs: 1, variants: ['9'] }]);
  assert.equal(courseEquipment(course, 9), null);
});

test('manpower and divisions are held steps to the shown hour, broken at unread losses, rosters and gaps', () => {
  const course = validateLandBattleCourse(wire([
    { hour: 10, terminal: false, sides: [{ manpower: 5, variants: [] }, { manpower: 1, variants: [] }] },
    { hour: 12, terminal: false, sides: 'Hourly losses not recorded' },
    { hour: 14, terminal: false, sides: [{ manpower: 9, variants: [] }, { manpower: 'Not in this recording', unknown: 'Not in this recording' }] },
  ], [run(10, 11, [3, 2]), run(12, 12, [0, 0], true), run(13, 14, [4, 1]), run(18, 20, [6, 2])]));
  const [attacker, defender] = courseManpower(course, 16);
  assert.deepEqual(attacker, [{ firstHour: 10, lastHour: 11, value: 5 }, { firstHour: 14, lastHour: 16, value: 9, breakBefore: true }]);
  assert.deepEqual(defender, [{ firstHour: 10, lastHour: 11, value: 1 }]);
  const [divisions] = courseDivisions(course, 19);
  assert.deepEqual(divisions, [{ firstHour: 10, lastHour: 11, value: 3 }, { firstHour: 13, lastHour: 14, value: 4, breakBefore: true },
    { firstHour: 18, lastHour: 19, value: 6, breakBefore: true }]);
});

test('a course chart spans zero to its largest value, however long the battle', () => {
  assert.equal(courseExtent([[], []]), null);
  assert.deepEqual(courseExtent([[{ firstHour: 0, lastHour: 0, value: 0 }], []]), [0, 1], 'an all-zero course still has a range');
  const long = Array.from({ length: 300_000 }, (_, hour) => ({ firstHour: hour, lastHour: hour, value: hour % 1000 }));
  assert.deepEqual(courseExtent([long, [...long, { firstHour: 300_000, lastHour: 300_000, value: 4321 }]]), [0, 4321]);
});


test('terrain replies keep each province and its reason; a name and a reason never both appear', () => {
  const reply = { provinces: [{ provinceId: 1, name: 'Mod hills', missing: null }, { provinceId: 2, name: null, missing: 'localisation mod_marsh is missing' }], warnings: [] };
  assert.deepEqual(validateLandBattleTerrain(reply), reply);
  for (const bad of [{ provinceId: 1, name: 'x', missing: 'y' }, { provinceId: 1, name: null, missing: null }, { provinceId: -1, name: 'x', missing: null }]) {
    assert.throws(() => validateLandBattleTerrain({ ...reply, provinces: [bad] }), /malformed/);
  }
});
