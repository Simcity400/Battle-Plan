import assert from 'node:assert/strict';
import test from 'node:test';
import { buildStack } from '../charts/stack.ts';
import { partsRaceAt, partsRaceLineRuns, partsRaceMixBands, partsRaceModel, partsRacePartName, partsRaceEntries, partsRaceSegmentAt,
  partsRaceTooltips, partsRaceAllZero, rankPartsRace, validatePartsRaceReply } from './partsRace.ts';
import type { PartsRaceReply } from './partsRace';

const reply = (): PartsRaceReply => ({ branchId: 'branch',
  parts: [{ key: 'mod aircraft', name: 'Mod aircraft', unresolved: false }, { key: ':variant/7', name: null, unresolved: true }],
  series: [{ tag: 'tag with , separators', segments: [{ firstHour: 10, lastHour: 15 }, { firstHour: 16, lastHour: 20 }], membership: null,
    runs: [{ firstHour: 10, lastHour: 11, startsSegment: true, total: 5, values: [3, 2] },
      { firstHour: 12, lastHour: 12, startsSegment: false, total: null, values: [null, null] },
      { firstHour: 13, lastHour: 14, startsSegment: false, total: 5, values: [null, null] },
      { firstHour: 16, lastHour: 20, startsSegment: true, total: 0, values: [0, 0] }] }], warnings: [] });

test('parts race lookups distinguish exact zero, missing totals and unrecorded hours', () => {
  const model = partsRaceModel(validatePartsRaceReply(reply()));
  const tag = model.tags[0];
  assert.equal(partsRaceAt(model, tag, 10.9)?.total, 5);
  assert.equal(partsRaceAt(model, tag, 12)?.total, null);
  assert.equal(partsRaceAt(model, tag, 15), null);
  assert.equal(partsRaceAt(model, tag, 20)?.total, 0);
  assert.equal(partsRaceAt(model, 'absent', 20), null);
  assert.equal(partsRaceSegmentAt(model, tag, 15), 10);
  assert.equal(partsRaceSegmentAt(model, tag, 16), 16);
  assert.equal(partsRaceSegmentAt(model, tag, 21), null);
});

test('parts race ranking excludes unknown totals, retains zero and breaks ties by UTF-8 tag', () => {
  const data = reply();
  data.series.push({ ...structuredClone(data.series[0]), tag: 'ä', runs: [{ firstHour: 10, lastHour: 20, startsSegment: true, total: 5, values: [3, 2] }] });
  const model = partsRaceModel(data);
  assert.deepEqual(rankPartsRace(model, model.tags, 10, 2).map(row => row.tag), ['tag with , separators', 'ä']);
  assert.deepEqual(rankPartsRace(model, model.tags, 12, 2).map(row => row.tag), ['ä']);
  assert.deepEqual(rankPartsRace(model, model.tags, 16, 2).map(row => row.at.total), [5, 0]);
});

test('parts race bars ease adjacent known counts and restart after unknown totals', () => {
  const data = reply();
  data.series[0].runs.splice(0, 1,
    { firstHour: 10, lastHour: 10, startsSegment: true, total: 4, values: [2, 2] },
    { firstHour: 11, lastHour: 11, startsSegment: false, total: 5, values: [3, 2] });
  const model = partsRaceModel(data);
  const entries = (hour: number) => partsRaceEntries(model, rankPartsRace(model, model.tags, hour, 10), false);
  assert.equal(entries(10)[0].stretch, entries(11)[0].stretch);
  assert.equal(entries(12).length, 0);
  assert.notEqual(entries(11)[0].stretch, entries(13)[0].stretch);
  assert.equal(entries(13)[0].stretch, 13);
});

test('unresolved content keeps its visible count; unknown breakdown hatches the exact total', () => {
  const model = partsRaceModel(reply());
  const complete = partsRaceEntries(model, rankPartsRace(model, model.tags, 10, 10), true)[0];
  assert.equal(complete.value, 5);
  assert.deepEqual(complete.parts, [{ id: 'mod aircraft', value: 3 }, { id: ':variant/7', value: 2 }]);
  assert.equal(partsRacePartName(model.reply.parts[1]), 'Part name unavailable');
  const unknown = partsRaceEntries(model, rankPartsRace(model, model.tags, 13, 10), true)[0];
  assert.equal(unknown.value, 5);
  assert.equal(unknown.hatched, 5);
  assert.equal(unknown.parts, undefined);
  const total = partsRaceEntries(model, rankPartsRace(model, model.tags, 13, 10), false)[0];
  assert.equal(total.hatched, undefined);
});

test('parts race lines and mix preserve gaps and identity breaks within group and date cuts', () => {
  const model = partsRaceModel(reply());
  const tag = model.tags[0];
  assert.deepEqual(partsRaceLineRuns(model, tag, null, { firstHour: 10, lastHour: 20 }), [
    { firstHour: 10, lastHour: 11, value: 5, breakBefore: true },
    { firstHour: 13, lastHour: 14, value: 5 },
    { firstHour: 16, lastHour: 20, value: 0, breakBefore: true },
  ]);
  assert.deepEqual(partsRaceLineRuns(model, tag, [{ firstHour: 14, lastHour: 18 }], { firstHour: 11, lastHour: 17 }), [
    { firstHour: 14, lastHour: 14, value: 5 }, { firstHour: 16, lastHour: 17, value: 0, breakBefore: true },
  ]);
  const bands = partsRaceMixBands(model, tag, null, { firstHour: 10, lastHour: 20 });
  assert.equal(bands[1][0].value, 2);
  const stack = buildStack(bands);
  assert.equal(stack.edges[stack.hours.indexOf(12)], null);
  assert.equal(stack.edges[stack.hours.indexOf(16)]?.at(-1)?.high, 0);
});

test('parts race tooltips show unknown parts and exact totals in every fitted form', () => {
  const model = partsRaceModel(reply());
  const forms = [...partsRaceTooltips({ model, name: 'Name at hour', at: partsRaceAt(model, model.tags[0], 13)!,
    date: 'Date', measure: 'Planes', ownership: 'Roster owner', colours: new Map() })];
  assert.ok(forms[0].rows.some(row => row.label === 'Part name unavailable' && row.value === 'Not known'));
  assert.ok(forms.every(form => form.rows.at(-1)?.value === '5'));
});

test('parts race accepts another domain and configured units with a reference total', () => {
  const data = reply();
  data.series[0].runs[0].referenceTotal = 10;
  const model = partsRaceModel(validatePartsRaceReply(data));
  const forms = [...partsRaceTooltips({ model, name: 'Country', at: partsRaceAt(model, model.tags[0], 10)!,
    date: 'Date', measure: 'Measure', ownership: 'Owner', colours: new Map(),
    formatValue: value => `${value} units`, referenceLabel: 'Reference' })];
  assert.ok(forms.every(form => form.rows.at(-1)?.value === '10 units'));
  assert.equal(partsRaceEntries(model, rankPartsRace(model, model.tags, 10, 10), false, value => `${value} units`)[0].label, '5 units');
});

test('parts race payload validation refuses malformed catalogues, counts, spans and uncovered runs', () => {
  assert.doesNotThrow(() => validatePartsRaceReply(reply()));
  const wrong = [null, [], { ...reply(), warnings: [3] }, { ...reply(), branchId: 2 }];
  for (const data of wrong) assert.throws(() => validatePartsRaceReply(data), /malformed/);
  const mutations: ((data: PartsRaceReply) => void)[] = [
    data => { data.parts.push({ ...data.parts[0] }); },
    data => { data.series.push(structuredClone(data.series[0])); },
    data => { data.series[0].runs[0].total = -1; },
    data => { data.series[0].runs[0].total = 4; },
    data => { data.series[0].runs[0].total = Number.MAX_SAFE_INTEGER + 1; },
    data => { data.series[0].runs[0].total = Number.MAX_SAFE_INTEGER; data.series[0].runs[0].values = [Number.MAX_SAFE_INTEGER, 1]; },
    data => { data.series[0].runs[0].values = [6, null]; },
    data => { data.series[0].runs[0].values[0] = .5; },
    data => { data.series[0].runs[0].lastHour = Infinity; },
    data => { data.series[0].runs[0].values.pop(); },
    data => { data.series[0].runs[1].firstHour = 11; },
    data => { data.series[0].segments[0].firstHour = 11; },
    data => { data.series[0].membership = [{ firstHour: 20, lastHour: 19 }]; },
  ];
  for (const mutation of mutations) {
    const data = reply(); mutation(data);
    assert.throws(() => validatePartsRaceReply(data), /malformed/);
  }
});

test('a tally ranked positive holds no country at zero, and all zero is told from unknown', () => {
  const data = reply();
  data.series.push({ ...structuredClone(data.series[0]), tag: 'AAA', runs: [{ firstHour: 10, lastHour: 20, startsSegment: true, total: 0, values: [0, 0] }] });
  const model = partsRaceModel(data);
  assert.deepEqual(rankPartsRace(model, model.tags, 10, 10, true).map(row => row.tag), ['tag with , separators']);
  assert.deepEqual(rankPartsRace(model, model.tags, 16, 10, true), [], 'zeros never fill a top N in tag order');
  assert.equal(partsRaceAllZero(model, model.tags, 16), true);
  assert.equal(partsRaceAllZero(model, model.tags, 10), false);
  assert.equal(partsRaceAllZero(model, ['tag with , separators'], 12), false, 'an unknown total is not a zero');
});
