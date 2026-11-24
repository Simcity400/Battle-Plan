import assert from 'node:assert/strict';
import test from 'node:test';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { landBattleRows } from './landBattlesData.ts';
import type { LandBattle, LandBattlesWindow } from './landBattlesData.ts';
import {
  battleOutcome,
  battleTooltip,
  courseDivisions,
  courseEquipment,
  courseExtent,
  courseManpower,
  drillChartHour,
  heldTerrain,
  missingTerrain,
  paintBattle,
  sameKnownSides,
  sideLook,
  validateLandBattleCourse,
  validateLandBattleTerrain,
  winners,
} from './landBattleCourse.ts';

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


const window = (rows: LandBattle[]): LandBattlesWindow => ({ window: 0, interval: { firstHour: 10, lastHour: 30 }, rows, observed: [[10, 30]], warnings: [], contentWarnings: [] });
const battle = (patch: Partial<LandBattle> = {}): LandBattle => ({ id: 'b', provinceId: 1, start: { hour: 10, bound: 'exact' }, end: { hour: 21, bound: 'exact' },
  endUnknownHour: null, counterBase: 0, priorPeak: null, priorSidePeaks: [null, null], result: { hour: 21, duration: 11, winner: { side: 1 },
    countries: [['A'], ['B', 'C', 'D']], countriesUnknown: [false, false] },
  losses: [{ hour: 21, terminal: true, equipmentRaw: ['250000', '-100000'], sides: [
    [{ label: 'Manpower lost', value: 30 }, { label: 'Equipment lost', value: 2.5 }], [{ label: 'Manpower lost', value: 7 }, { label: 'Equipment lost', value: -1 }]] }],
  runs: [run(10, 20, [3, 2])], ...patch });
const names = (tag: string) => ({ name: `Country ${tag}` });

test('the outcome names the winners; endings and verdicts that were not read stay unknown in plain words', () => {
  const [row] = landBattleRows(new Map([[0, window([battle()])]]), 21, () => true, true);
  assert.deepEqual(battleOutcome(row), { kind: 'won', side: 1 });
  const looks = row.sides.map(side => sideLook(side, names));
  assert.equal(looks[0].name, 'Country A');
  assert.equal(looks[1].name, null);
  assert.equal(winners(looks[1], 'Defenders'), 'Country B and 2 more');
  assert.equal(winners(looks[0], 'Attackers'), 'Country A');
  assert.deepEqual(battleOutcome(landBattleRows(new Map([[0, window([battle()])]]), 20, () => true, true)[0]), { kind: 'ongoing' });
  const unread = landBattleRows(new Map([[0, window([battle({ end: { unknown: 'Ending not recorded' }, endUnknownHour: 21, result: null })])]]), 21, () => true, true)[0];
  assert.deepEqual(battleOutcome(unread), { kind: 'unknown', text: 'Ending not recorded' });
  const verdict = landBattleRows(new Map([[0, window([battle({ result: { ...battle().result!, winner: { unknown: 'No winning side' } } })])]]), 21, () => true, true)[0];
  assert.deepEqual(battleOutcome(verdict), { kind: 'unknown', text: 'No winning side' });

  const unreadHourWinner = landBattleRows(new Map([[0, window([battle({
    end: { unknown: 'Ending hour not recorded' }, endUnknownHour: 25, settledHour: 25,
    runs: [run(10, 24, [3, 2])],
    result: { hour: null, duration: 15, winner: { side: 1 }, countries: [['A'], ['B', 'C', 'D']], countriesUnknown: [false, false] },
  })])]]), 40, () => true, true)[0];
  assert.deepEqual(battleOutcome(unreadHourWinner), { kind: 'won', side: 1 }, 'a missing end hour keeps a recorded winner');
  const unreadHourLooks = unreadHourWinner.sides.map(side => sideLook(side, names));
  const unreadHourTip = battleTooltip(unreadHourWinner, unreadHourLooks, null, ['blue', 'orange'], false);
  assert.equal(unreadHourTip.rows.find(entry => entry.label === 'Ended')?.value, 'Ending hour not recorded');
  assert.equal(unreadHourTip.rows.at(-1)?.value, 'Country B and 2 more won');
  assert.equal(unreadHourWinner.settledHour, 25);
  assert.equal(drillChartHour(unreadHourWinner, 40), 25, 'drill chart caps at settlement bound when end hour is unread');
  const course = validateLandBattleCourse(wire([
    { hour: 24, terminal: false, sides: [{ manpower: 12, variants: [] }, { manpower: 4, variants: [] }] },
  ], [run(10, 24, [3, 2])]));
  const [attackers, defenders] = courseManpower(course, drillChartHour(unreadHourWinner, 40));
  assert.deepEqual(attackers, [{ firstHour: 24, lastHour: 25, value: 12 }]);
  assert.deepEqual(defenders, [{ firstHour: 24, lastHour: 25, value: 4 }]);
});

test('the tooltip is one compact summary: no per-design rows, whole equipment and one dated loss note', () => {
  const [row] = landBattleRows(new Map([[0, window([battle()])]]), 21, () => true, true);
  const looks = row.sides.map(side => sideLook(side, names));
  const tip = battleTooltip({ ...row, place: { ...row.place, name: 'Mod town' } }, looks, 'Mod hills', ['blue', 'orange'], false);
  assert.equal(tip.title, 'Mod town');
  assert.equal(tip.note, `Final losses, read ${formatDateTime(21)}`);
  assert.deepEqual(tip.rows.find(entry => entry.label === 'Terrain'), { label: 'Terrain', value: 'Mod hills' });
  assert.deepEqual(tip.rows.filter(entry => entry.label === 'Equipment lost').map(entry => entry.value), ['3', '−1']);
  assert.ok(tip.rows.every(entry => !entry.label.startsWith('Equipment lost ·') && !String(entry.value ?? '').includes('as of')));
  assert.equal(tip.rows.at(-1)?.value, 'Country B and 2 more won');
  assert.equal(tip.rows.find(entry => entry.label === 'Length')?.value, '11 hours');
  assert.equal(battleTooltip({ ...row, place: { ...row.place, name: null } }, looks, null, ['blue', 'orange'], true).title, 'Loading…');
  assert.equal(battleTooltip({ ...row, place: { ...row.place, name: null } }, looks, null, ['blue', 'orange'], false).title, 'Unnamed province');
  const missingTip = battleTooltip({ ...row, provinceId: null, place: { ...row.place, name: null } }, looks, null, ['blue', 'orange'], false);
  assert.equal(missingTip.title, 'Province not recorded');

  const unreadWinnerRow = landBattleRows(new Map([[0, window([battle({ result: { hour: 21, duration: 11, winner: { side: 0 }, countries: [[], ['B']], countriesUnknown: [true, false] } })])]]), 21, () => true, true)[0];
  const unreadWinnerLooks = unreadWinnerRow.sides.map(side => sideLook(side, names));
  assert.equal(winners(unreadWinnerLooks[0], 'Attackers'), 'Attackers', 'unread winning side displays the side, never earlier run countries');
  const unreadWinnerTip = battleTooltip(unreadWinnerRow, unreadWinnerLooks, null, ['blue', 'orange'], false);
  assert.equal(unreadWinnerTip.rows.at(-1)?.value, 'Attackers won');
});

test('terrain replies keep each province and its reason; a name and a reason never both appear', () => {
  const reply = { provinces: [{ provinceId: 1, name: 'Mod hills', missing: null }, { provinceId: 2, name: null, missing: 'localisation mod_marsh is missing' }], warnings: [] };
  assert.deepEqual(validateLandBattleTerrain(reply), reply);
  for (const bad of [{ provinceId: 1, name: 'x', missing: 'y' }, { provinceId: 1, name: null, missing: null }, { provinceId: -1, name: 'x', missing: null }]) {
    assert.throws(() => validateLandBattleTerrain({ ...reply, provinces: [bad] }), /malformed/);
  }
});

test('terrain names read are held for one client, recording and content, and asked for again after any change', () => {
  const client = {}, source = { id: 'source', revision: '1' };
  const held = heldTerrain(null, client, source, 0);
  held.names.set(1, { provinceId: 1, name: 'Mod hills', missing: null });
  assert.deepEqual(missingTerrain(held, [2, 1, 2]), [2], 'a held province is not asked for again; a drawn one once');
  assert.equal(heldTerrain(held, client, { ...source }, 0), held, 'the same scope keeps what was read');
  for (const changed of [heldTerrain(held, client, source, 1), heldTerrain(held, client, { ...source, revision: '2' }, 0),
    heldTerrain(held, client, { ...source, id: 'other' }, 0), heldTerrain(held, {}, source, 0)]) {
    assert.notEqual(changed, held);
    assert.deepEqual(missingTerrain(changed, [2, 1]), [1, 2]);
  }
});


test('partial battle losses update the known side during playback without bars or touching the unread side', () => {
  const partialWindow = window([battle({
    end: null,
    result: null,
    runs: [run(10, 25, [3, 2])],
    losses: [
      { hour: 12, terminal: false, sides: [[{ label: 'Manpower lost', value: 5 }], [{ label: 'Manpower lost', value: null, unknown: 'Defender losses not recorded' }]] },
      { hour: 16, terminal: false, sides: [[{ label: 'Manpower lost', value: 9 }], [{ label: 'Manpower lost', value: null, unknown: 'Defender losses not recorded' }]] },
    ],
  })]);
  const windows = new Map([[0, partialWindow]]);
  const at12 = landBattleRows(windows, 12, () => true, true)[0];
  const at16 = landBattleRows(windows, 16, () => true, true)[0];
  assert.equal(sameKnownSides(at12, at16), true);

  const attackerValue = { textContent: '5', classList: { contains: () => false }, firstElementChild: null };
  const defenderValue = { textContent: 'Not recorded', classList: { contains: (name: string) => name === 'is-muted' }, firstElementChild: null };
  const sub = { textContent: '' };
  const host = {
    contains: (node: unknown) => node === attackerValue || node === defenderValue || node === sub,
    querySelectorAll: (selector: string) => selector === '.lb-losses .log-value' ? [attackerValue, defenderValue] : [],
    querySelector: (selector: string) => selector === '.lb-place small' ? sub : null,
  } as unknown as HTMLElement;

  paintBattle(host, at12, 'Plains');
  assert.equal(attackerValue.textContent, '5');
  assert.equal(defenderValue.textContent, 'Not recorded');
  paintBattle(host, at16, 'Plains');
  assert.equal(attackerValue.textContent, '9', 'known side updates during playback when the other side is unread');
  assert.equal(defenderValue.textContent, 'Not recorded', 'unread side stays untouched');
});
