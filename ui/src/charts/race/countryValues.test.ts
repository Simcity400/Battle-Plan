import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValueAt, countryValueEntries, countryValueMix,
  countryValuePartsReply, countryValuesModel, countryValueSegmentAt, rankCountryValues,
  validCountryValues } from './countryValues.ts';
import type { CountryValuesReply } from './countryValues';
import { countryRaceItems } from './countryRace.ts';

const REPLY: CountryValuesReply = {
  types: [{ key: 'mod group', name: 'Mod group' }, { key: '?unresolved', name: null }],
  series: [
    { tag: 'tag with spaces', segments: [{ firstHour: 1, lastHour: 4 },
      { firstHour: 6, lastHour: 7 }, { firstHour: 8, lastHour: 8 }], membership: null,
      runs: [[1, 10, 2], [3, null, 4], [6, 12, 7], [8, 12, null]],
      mix: [{ firstHour: 1, lastHour: 2, values: [6, 4] },
        { firstHour: 3, lastHour: 4, values: [null, null] },
        { firstHour: 6, lastHour: 7, values: [7, null] },
        { firstHour: 8, lastHour: 8, values: [7, 5] }],
    },
    { tag: 'a', segments: [{ firstHour: 1, lastHour: 8 }], membership: null,
      runs: [[1, 0, 8]], mix: [{ firstHour: 1, lastHour: 8, values: [0, 0] }] },
  ],
};

test('compact country histories reject malformed, overlapping and unobserved totals or mix', () => {
  assert.equal(validCountryValues(REPLY), true);
  for (const mutate of [
    (reply: CountryValuesReply) => { reply.series[0].runs[1][0] = 2; },
    (reply: CountryValuesReply) => { reply.series[0].mix![0].values = [1]; },
    (reply: CountryValuesReply) => { reply.series[0].runs[0][1] = Number.NaN; },
    (reply: CountryValuesReply) => { reply.series[0].runs[0][2] = 0; },
    (reply: CountryValuesReply) => { reply.series[0].mix![1].firstHour = 2; },
    (reply: CountryValuesReply) => { reply.series[0].segments[0].lastHour = 1; },
    (reply: CountryValuesReply) => { reply.series[0].membership = [{ firstHour: 2, lastHour: 8 }]; },
    (reply: CountryValuesReply) => { reply.types[1].key = reply.types[0].key; },
  ]) {
    const invalid = structuredClone(REPLY);
    mutate(invalid);
    assert.equal(validCountryValues(invalid), false);
  }
  const objectRuns = { ...REPLY, series: [{ ...REPLY.series[1], runs: [{
    firstHour: 1, lastHour: 8, total: 0, values: [0, 0], startsSegment: true, readHour: 1,
  }] }] };
  assert.equal(validCountryValues(objectRuns), false);
  assert.equal(validCountryValues(null), false);
});

test('hourly read dating survives compressed equal reads; unknown and absent never rank as zero', () => {
  const model = countryValuesModel(REPLY);
  assert.equal(countryValueAt(model, 'tag with spaces', 2)?.readHour, 2);
  assert.equal(countryValueAt(model, 'tag with spaces', 3)?.total, null);
  assert.equal(countryValueAt(model, 'tag with spaces', 5), null);
  assert.deepEqual(rankCountryValues(model, model.tags, 3, 10).map(entry => [entry.tag, entry.at.total]), [['a', 0]]);
  assert.deepEqual(rankCountryValues(model, model.tags, 5, 10).map(entry => entry.tag), ['a']);
  assert.deepEqual(rankCountryValues(model, model.tags, 6, 1).map(entry => entry.tag), ['tag with spaces']);
  assert.equal(countryValueSegmentAt(model, 'tag with spaces', 7), 6);
  assert.equal(countryValueSegmentAt(model, 'tag with spaces', 5), null);
  const recovered = structuredClone(REPLY);
  recovered.series[0].runs[2][0] = 5;
  recovered.series[0].segments = [{ firstHour: 1, lastHour: 8 }];
  assert.equal(countryValueAt(countryValuesModel(recovered), 'tag with spaces', 6)?.stretch, 5);
  const tied = structuredClone(REPLY);
  tied.series[0].tag = 'A';
  tied.series[0].runs[0][1] = 0;
  assert.deepEqual(rankCountryValues(countryValuesModel(tied), ['a', 'A'], 1, 2).map(entry => entry.tag), ['A', 'a']);
});

test('mix retains gaps, identities, and member/range boundaries', () => {
  const model = countryValuesModel(REPLY);
  const mix = countryValueMix(model, 'tag with spaces', [{ firstHour: 6, lastHour: 8 }], { firstHour: 7, lastHour: 8 });
  assert.deepEqual(mix, [[{ firstHour: 7, lastHour: 7, value: 7 }, { firstHour: 8, lastHour: 8, value: 7, breakBefore: true }],
    [{ firstHour: 8, lastHour: 8, value: 5, breakBefore: true }]]);
});

test('split races hatch unknown attribution while preserving the independently exact total', () => {
  const model = countryValuesModel(REPLY);
  const entries = countryValueEntries(model, rankCountryValues(model, model.tags, 6, 1), true, String);
  assert.deepEqual(entries[0].parts, [{ id: 'mod group', value: 7 }, { id: '', value: 0, hatched: 5 }]);
  assert.equal(entries[0].value, 12);
  const items = countryRaceItems(entries, new Map([['mod group', 'type']]), () => 'country', 'unknown');
  assert.equal(items[0].segments.reduce((sum, part) => sum + part.value + (part.hatched ?? 0), 0), 12);
  const totalsOnly = { ...REPLY, types: [], series: REPLY.series.map(series => ({ ...series, mix: undefined })) };
  assert.equal(validCountryValues(totalsOnly), true);
  assert.equal(countryValueEntries(countryValuesModel(totalsOnly), rankCountryValues(countryValuesModel(totalsOnly), ['a'], 1, 1), true, String)[0].parts, undefined);
});

test('compact hourly totals and separate composition preserve exact reads, groups and identity breaks', () => {
  const reply: CountryValuesReply = { types: [{ key: 'authored', name: 'Authored' }], series: [{
    tag: 'country', membership: null, segments: [{ firstHour: 1, lastHour: 3 }, { firstHour: 4, lastHour: 5 }],
    runs: [[1, 2, 3], [4, 2, 5]], mix: [{ firstHour: 1, lastHour: 3, values: [2] }, { firstHour: 4, lastHour: 5, values: [2] }],
  }] };
  assert.equal(validCountryValues(reply), true);
  const model = countryValuesModel(reply);
  assert.deepEqual(countryValueAt(model, 'country', 5)?.values, [2]);
  assert.equal(countryValueAt(model, 'country', 5)?.readHour, 5);
  assert.equal(validCountryValues({ ...reply, series: [{ ...reply.series[0], runs: [[1, 2, 0]] }] }), false);
  assert.equal(validCountryValues({ ...reply, series: [{ ...reply.series[0], mix: [{ firstHour: 1, lastHour: 6, values: [2] }] }] }), false);
});

test('a missing separate group read keeps the exact total and hatches its unknown split', () => {
  const reply: CountryValuesReply = { types: [{ key: 'authored', name: 'Authored' }], series: [{
    tag: 'country', membership: null, segments: [{ firstHour: 1, lastHour: 3 }],
    runs: [[1, 4, 3]], mix: [{ firstHour: 1, lastHour: 1, values: [4] }],
  }] };
  assert.equal(validCountryValues(reply), true);
  const model = countryValuesModel(reply);
  const at = countryValueAt(model, 'country', 2)!;
  assert.equal(at.total, 4);
  assert.deepEqual(at.values, [null]);
  assert.deepEqual(countryValueEntries(model, [{ tag: 'country', at: { ...at, total: 4 } }], true, String)[0].parts,
    [{ id: '', value: 0, hatched: 4 }]);
});

test('the parts race shape joins boundaries without filling gaps or expanding hours', () => {
  const reply = { branchId: '1', warnings: ['kept'], types: [{ key: 'unknown:unit', name: 'Unit' }],
    series: [{ tag: 'opaque', membership: [{ firstHour: 1, lastHour: 4 }, { firstHour: 6, lastHour: 1_000_000 }],
      segments: [{ firstHour: 1, lastHour: 4 }, { firstHour: 6, lastHour: 1_000_000 }],
      runs: [[1, 4, 2], [3, 6, 4], [6, 6, 1_000_000]],
      mix: [{ firstHour: 1, lastHour: 1, values: [4] }, { firstHour: 3, lastHour: 4, values: [6] },
        { firstHour: 6, lastHour: 1_000_000, values: [null] }] }] } satisfies
    CountryValuesReply & { branchId: string; warnings: string[] };
  const parts = countryValuePartsReply(reply, type => type.key.startsWith('unknown:'));
  assert.deepEqual(parts.parts, [{ key: 'unknown:unit', name: 'Unit', unresolved: true }]);
  assert.deepEqual(parts.series[0].runs, [
    { firstHour: 1, lastHour: 1, total: 4, rawTotal: undefined, rawValues: undefined, startsSegment: true, values: [4] },
    { firstHour: 2, lastHour: 2, total: 4, rawTotal: undefined, rawValues: undefined, startsSegment: false, values: [null] },
    { firstHour: 3, lastHour: 4, total: 6, rawTotal: undefined, rawValues: undefined, startsSegment: false, values: [6] },
    { firstHour: 6, lastHour: 1_000_000, total: 6, rawTotal: undefined, rawValues: undefined, startsSegment: true, values: [null] },
  ]);
  assert.deepEqual(parts.series[0].membership, reply.series[0].membership);
  assert.deepEqual(parts.warnings, ['kept']);
});
