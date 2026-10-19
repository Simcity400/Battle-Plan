import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValueAt, countryValuesModel } from '../../charts/race/countryValues.ts';
import { createWidget } from '../../dashboard/model.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import type { ArmySizeReply } from '../../generated/dashboard';
import '../definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { armyMeasure, armySizeRequest, armyTooltips, armyWindows, formatArmyValue, formatArmyRaw,
  measureChoices, mergeArmyWindows, validateArmySizeReply, validateArmyWindow } from './armySizeData.ts';

const REPLY: ArmySizeReply = {
  branchId: 'synthetic', available: true, measures: ['divisions', 'strength', 'battalions'], warnings: [],
  windows: [], window: 0, interval: { firstHour: 100, lastHour: 105 },
  types: [{ key: 'modded_group', name: 'Modded unit group' }],
  series: [{ tag: 'opaque tag', membership: null, segments: [{ firstHour: 100, lastHour: 105 }],
    runs: [[100, 4, 105]], mix: [{ firstHour: 100, lastHour: 105, values: [4] }] }],
};

test('Army size validates capabilities alongside the shared history shape', () => {
  assert.equal(validateArmySizeReply(REPLY), REPLY);
  assert.throws(() => validateArmySizeReply({ ...REPLY, measures: ['unknown'] }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, available: 1 }));
  assert.throws(() => validateArmySizeReply({ ...REPLY, warnings: [1] }));
  assert.equal(validateArmySizeReply({ ...REPLY, templateSections: ['Unused template', 'Staff and garrison', 'HQ deployment'] }).templateSections?.length, 3);
  assert.throws(() => validateArmySizeReply({ ...REPLY, templateSections: [17] }));
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

test('new Army size widgets share the top 10, race, date-off and owner defaults', () => {
  const widget = createWidget('army-size', 'main', 'army');
  const options = resolveOptions(OPTIONS, widget.options);
  assert.deepEqual(widget.countries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  assert.equal(widget.time.mode, 'campaign');
  assert.equal(sectionFields(OPTIONS, 'options', { ...options, view: 'mix' }).some(field => field.key === 'measure'), false);
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([options.view, options.measure, options.countFor, options.byType, options.dateOverlay], ['race', '', 'owner', false, false]);
});

test('requests preserve opaque country tags, owner/host, recorded measures and mix attribution', () => {
  assert.deepEqual(armySizeRequest('synthetic', { tags: ['opaque tag'] }, 'battalions', 'host', true),
    { branchId: 'synthetic', tags: ['opaque tag'], top: null, measure: 'battalions', countFor: 'host', byType: true, window: null });
  assert.deepEqual(armySizeRequest('synthetic', { tags: [], top: 30 }, 'strength', 'owner', false, 7),
    { branchId: 'synthetic', tags: [], top: 30, measure: 'strength', countFor: 'owner', byType: false, window: 7 });
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
