import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValueAt, countryValuesModel } from '../../charts/race/countryValues.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { ArmySizeReply } from './armySizeData';
import '../definitions.ts';
import { armyAbsent, armyMeasure, armySizeRequest, armyTooltips, armyWindows, formatArmyValue, formatArmyRaw, formatArmyRawFast,
  measureChoices, mergeArmyWindows, offersMeasure, validateArmySizeReply, validateArmyWindow } from './armySizeData.ts';

const REPLY: ArmySizeReply = {
  branchId: 'synthetic', available: true, measures: ['divisions', 'strength', 'battalions'], warnings: [],
  windows: [], window: 0, interval: { firstHour: 100, lastHour: 105 },
  types: [{ key: 'modded_group', name: 'Modded unit group' }],
  series: [{ tag: 'opaque tag', membership: null, segments: [{ firstHour: 100, lastHour: 105 }],
    runs: [[100, 4, 105]], mix: [{ firstHour: 100, lastHour: 105, values: [4] }] }],
};

test('Army size validates capabilities alongside the shared history shape', () => {
  assert.equal(validateArmySizeReply(REPLY), REPLY);
  const asked = { ...REPLY, series: [...REPLY.series, { ...REPLY.series[0], tag: undefined, country: 5 }, { ...REPLY.series[0], tag: undefined, country: '5' }]
    .map(({ tag, ...series }) => tag === undefined ? series : { ...series, country: tag }) };
  assert.deepEqual(validateArmySizeReply(asked).series.map(series => series.tag), ['opaque tag', countryKey(5), '5'], 'a slot and a mod tag spelled as its number stay apart');
  assert.throws(() => validateArmySizeReply({ ...REPLY, measures: ['unknown'] }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, available: 1 }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, warnings: [1] }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, window: -1 }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, interval: null }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, types: [{ key: 'group', name: null }] }));
  assert.throws(() => validateArmySizeReply(null));
});

test('only recorded measures are offered; divisions are the fallback until manpower capture lands', () => {
  assert.deepEqual(measureChoices({ ...REPLY, measures: ['divisions'] }), [{ value: 'divisions', label: 'Number of divisions' }]);
  assert.deepEqual(measureChoices(undefined), []);
  assert.equal(armyMeasure('', REPLY.measures), 'divisions');
  assert.equal(armyMeasure('', ['strength', 'divisions']), 'divisions');
  assert.equal(armyMeasure('strength', REPLY.measures), 'strength');
  assert.equal(armyMeasure('strength', ['divisions']), 'divisions');
  assert.equal(armyMeasure('', ['strength']), 'strength');
});

test('only the measures the recording reports are offered, and an army it does not record is absent', () => {
  const partial: ArmySizeReply = { ...REPLY, available: false, measures: ['strength'] };
  assert.equal(validateArmySizeReply(partial), partial);
  assert.equal(armyMeasure('manpower', partial.measures), 'strength', 'a measure not recorded is not chosen');
  assert.equal(armyMeasure('', partial.measures), 'strength', 'a default is a reported measure');
  assert.equal(offersMeasure(partial, 'manpower'), false);
  assert.equal(offersMeasure(partial, 'strength'), true);
  assert.equal(armyAbsent(partial), true);
  assert.equal(armyAbsent(REPLY), false);
});


test('requests preserve opaque country tags, owner/host, recorded measures and mix attribution', () => {
  assert.deepEqual(armySizeRequest('synthetic', { tags: ['opaque tag', countryKey(1499), '1499'] }, 'battalions', 'host', true),
    { branchId: 'synthetic', countries: ['opaque tag', 1499, '1499'], top: null, measure: 'battalions', countFor: 'host', byType: true, window: null },
    'a picked tag is asked as a tag, the dashboard country by its slot');
  assert.deepEqual(armySizeRequest('synthetic', { tags: [], top: 30 }, 'strength', 'owner', false, 7),
    { branchId: 'synthetic', countries: [], top: 30, measure: 'strength', countFor: 'owner', byType: false, window: 7 });
});

test('land tooltips name content groups and the exact hourly read without inventing a missing mix', () => {
  const model = countryValuesModel(REPLY);
  const at = countryValueAt(model, 'opaque tag', 104)!;
  const [full, compact] = [...armyTooltips(model, 'Name at hour 104', at, 104, 'battalions', new Map([['modded_group', 'palette']]))];
  assert.equal(full.title, 'Name at hour 104');
  assert.deepEqual(full.rows.map(row => [row.label, row.value]), [['Modded unit group', '4'], ['Battalions', '4']]);
  assert.equal(full.note, undefined);
  assert.deepEqual(compact.rows, [full.rows[1]]);
  assert.equal(formatArmyValue(1234.00001), '1,234.00001');
  const absent = { ...REPLY, types: [], series: REPLY.series.map(series => ({ ...series, mix: undefined })) };
  const [total] = [...armyTooltips(countryValuesModel(absent), 'Country', { ...at, values: [] }, 104, 'strength', new Map())];
  assert.deepEqual(total.rows.map(row => row.label), ['Combined strength (hit points)']);
});

const MANIFEST: ArmySizeReply = { ...REPLY, types: [], series: [], window: null, interval: null,
  windows: [{ firstHour: 100, lastHour: 105 }, { firstHour: 106, lastHour: 107 },
    { firstHour: 108, lastHour: 110 }] };

test('Army windows come from recording spans and prefetch never becomes required', () => {
  assert.deepEqual(armyWindows(MANIFEST, { firstHour: 102, lastHour: 104 }, 104, false), [0]);
  assert.deepEqual(armyWindows(MANIFEST, { firstHour: 102, lastHour: 104 }, 104, true), [0, 1]);
  assert.deepEqual(armyWindows(MANIFEST, { firstHour: 108, lastHour: 109 }, 104, true), [0, 1, 2]);
  assert.deepEqual(armyWindows(MANIFEST, { firstHour: 108, lastHour: 109 }, 104, false), [0, 2]);
  assert.deepEqual(armyWindows(MANIFEST, { firstHour: 111, lastHour: 112 }, 112, true), []);
});

test('viewport zoom and pan release outside windows; the margin scales with the visible span', () => {
  const manifest = { ...MANIFEST, windows: Array.from({ length: 10 }, (_, index) => ({
    firstHour: index * 100, lastHour: index * 100 + 99,
  })) };
  assert.deepEqual(armyWindows(manifest, { firstHour: 400, lastHour: 599 }, 450, false), [4, 5]);
  assert.deepEqual(armyWindows(manifest, { firstHour: 400, lastHour: 599 }, 450, false, true), [3, 4, 5, 6]);
  assert.deepEqual(armyWindows(manifest, { firstHour: 750, lastHour: 775 }, 450, false, true), [4, 7]);
  assert.deepEqual(armyWindows(manifest, { firstHour: 0, lastHour: 999 }, 450, false, true), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
});

test('Army window validation refuses another interval, branch and values outside its recorded block', () => {
  assert.equal(validateArmyWindow(REPLY, MANIFEST), REPLY);
  assert.throws(() => validateArmyWindow({ ...REPLY, branchId: 'another' }, MANIFEST));
  assert.throws(() => validateArmyWindow({ ...REPLY, window: 1 }, MANIFEST));
  assert.throws(() => validateArmyWindow({ ...REPLY,
    series: REPLY.series.map(series => ({ ...series, runs: [[99, 4, 105]],
      segments: [{ firstHour: 99, lastHour: 105 }] })) }, MANIFEST));
  assert.throws(() => validateArmyWindow({ ...REPLY,
    series: REPLY.series.map(series => ({ ...series,
      mix: [{ firstHour: 100, lastHour: 106, values: [4] }] })) }, MANIFEST));
  assert.throws(() => validateArmyWindow({ ...REPLY,
    series: REPLY.series.map(series => ({ ...series,
      membership: [{ firstHour: 99, lastHour: 105 }] })) }, MANIFEST));
});

test('Army windows join in recording order, remap authored groups and retain missing windows as gaps', () => {
  const later: ArmySizeReply = { ...REPLY, window: 2, interval: { firstHour: 108, lastHour: 110 },
    types: [{ key: 'unknown:support', name: 'Support (unknown-group)' }],
    series: [{ tag: 'opaque tag', membership: null, segments: [{ firstHour: 100, lastHour: 110 }],
      runs: [[108, 2, 110]], mix: [{ firstHour: 108, lastHour: 110, values: [2] }] }] };
  const joined = mergeArmyWindows(MANIFEST, new Map([[2, later], [0, REPLY]]), [2, 0, 1]);
  assert.equal(validateArmySizeReply(joined), joined);
  const model = countryValuesModel(joined);
  assert.equal(countryValueAt(model, 'opaque tag', 107), null);
  assert.deepEqual(countryValueAt(model, 'opaque tag', 104)?.values, [4, 0]);
  assert.deepEqual(countryValueAt(model, 'opaque tag', 108)?.values, [0, 2]);
  assert.deepEqual(joined.series[0].runs, [[100, 4, 105], [108, 2, 110]]);
  assert.deepEqual(joined.series[0].segments, [{ firstHour: 100, lastHour: 110 }]);
  assert.equal(REPLY.series[0].segments[0].lastHour, 105);
  assert.deepEqual(mergeArmyWindows(MANIFEST, new Map([[2, later]]), [0]).series, []);
});


test('manpower is the available default and HP labels retain exact raw units', () => {
  assert.equal(armyMeasure('', ['divisions', 'manpower']), 'manpower');
  assert.equal(formatArmyRaw('9007199254740986', 'strength'), '90,071,992,547.40986');
  assert.equal(formatArmyRaw('9007199254740987', 'strength'), '90,071,992,547.40987');
  assert.equal(formatArmyRaw('9223372036854775808', 'strength'), '92,233,720,368,547.75808');
});

test('large adjacent HP totals remain distinct in exact tooltips and ranking', () => {
  const raw = ['9007199254740986', '9007199254740987'];
  const reply: ArmySizeReply = { ...REPLY, types: [], series: raw.map((value, index) => ({
    tag: `country${index}`, membership: null, segments: [{ firstHour: 100, lastHour: 105 }],
    runs: [[100, Number(value) / 100000, 105, value]],
  })) };
  validateArmySizeReply(reply);
  const model = countryValuesModel(reply);
  const labels = raw.map((_, index) => [...armyTooltips(model, 'Country', countryValueAt(model, `country${index}`, 100)!, 100, 'strength', new Map())][0].rows[0].value);
  assert.deepEqual(labels, ['90,071,992,547.40986', '90,071,992,547.40987']);
});

test('a race label written fast is the label formatArmyRaw writes', () => {
  const values = [0n, 1n, 7n, 999n, 1000n, 1001n, 12_345n, 100_000n, 123_456_789n, 9_007_199_254_740_991n, 9_007_199_254_740_992n, -1n, -100_000n, -123_456_789n];
  for (let at = 0; at < 2000; at++) values.push(BigInt(Math.floor((at * 2_654_435_761) % 10 ** (1 + at % 15))) * (at % 7 === 0 ? -1n : 1n));
  for (const measure of ['divisions', 'strength', 'battalions', 'manpower'] as const)
    for (const raw of values) assert.equal(formatArmyRawFast(raw, measure), formatArmyRaw(raw.toString(), measure), `${raw} ${measure}`);
});
