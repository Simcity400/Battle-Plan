import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { MilitaryAirAtReply, MilitaryAirAtRow } from '../../generated/dashboard';
import { lossCausesModel, militaryAirModel, type MilitaryAirAtOptions } from './militaryAirAt.ts';

const identity: IdentityProvider = {
  resolve(kind, key, atHour) {
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      tag: { status: 'ready', value: key },
      name: { status: 'ready', value: `${key} name` },
      colour: { status: 'ready', value: '#4f86f7' },
      flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000],
      nextChangeAt: Infinity,
    };
  },
};

const AIR: MilitaryAirAtOptions = {
  family: 'airLosses', scale: 100_000, digits: 5, shownDecimals: 0, leftHeader: 'Own losses', rightHeader: 'Enemy aircraft destroyed', unit: 'Aircraft', ratio: true,
  group: false, none: 'No aircraft lost yet.', filters: { regions: [], causes: [] },
};
const PAIRED: MilitaryAirAtOptions = { ...AIR, leftHeader: 'Own losses', rightHeader: 'Destroyed', paired: { solid: 'Destroyed (solid)', outline: 'Own losses (outline)' } };

const aircraft = (count: number) => String(BigInt(count) * 100_000n);
const cause = (id: number, taken: string, name: string | null = id === 1 ? 'Aircraft combat' : id === 3 ? 'Air accidents' : null) =>
  ({ cause: id, name, missing: name ? null : `Air loss cause ${id} has no proved display meaning`, taken });
const row = (patch: Partial<MilitaryAirAtRow> & Pick<MilitaryAirAtRow, 'tag' | 'identity'>): MilitaryAirAtRow => ({
  segmentFirstHour: 0, dealt: '0', taken: '0', complete: true, partial: null, ratioHundredths: null, causes: [], ...patch,
});

const reply = (patch: Partial<MilitaryAirAtReply>): MilitaryAirAtReply => ({
  family: 'airLosses', hour: 500, countedFrom: 0, validUntil: 520, missing: null, rankMissing: null, unattributed: ['0', '0'], absent: [], warnings: [],
  rows: [
    row({ tag: 'GER', identity: '7:1', dealt: aircraft(4453), taken: aircraft(2474), ratioHundredths: '180', causes: [cause(1, aircraft(2000)), cause(3, aircraft(474))] }),
    row({ tag: 'ENG', identity: '8:2', dealt: aircraft(1439), taken: aircraft(2085), ratioHundredths: '69', causes: [cause(1, aircraft(2085))] }),
  ],
  ...patch,
});

const view = (data: MilitaryAirAtReply) => createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', data]]), identity });
const model = (data: MilitaryAirAtReply, options = AIR) => militaryAirModel(view(data), options);

test('the product order and ratio are drawn as given, each row keyed and drawn by its recorded slot', () => {
  const drawn = model(reply({}));
  assertBarsModelValid(drawn, 500);
  const rows = drawn.rows as BarsDataRow[];
  assert.deepEqual(rows.map(item => [item.key, item.subject]), [['7:1', countryKey(7)], ['8:2', countryKey(8)]]);
  assert.deepEqual([formatBarsValueField(rows[0]!.left!.value).text, formatBarsValueField(rows[0]!.right!.value).text], ['2,474', '4,453']);
  assert.deepEqual(rows.map(item => item.readout?.primary), ['1.80 : 1', '0.69 : 1'], 'the ratio is the product\'s');
});

test('R2-K6: aircraft are cut whole on screen from the recorded integer, 0.4 is never drawn as zero, and the tooltip keeps 1.23456', () => {
  const drawn = model(reply({ rows: [row({ tag: 'GER', identity: '7:1', dealt: '40000', taken: '123456' })] }), PAIRED);
  const first = drawn.rows[0] as BarsDataRow;
  assert.deepEqual([formatBarsValueField(first.left!.value).text, formatBarsValueField(first.left!.value).exact], ['1', '1.23456']);
  assert.equal(first.readout?.primary, '<1 / 1');
  const lost = lossCausesModel(view(reply({ rows: [row({ tag: 'GER', identity: '7:1', taken: '60000', causes: [cause(1, '60000')] })] })), AIR);
  assert.equal((lost.rows[0] as BarsDataRow).readout?.primary, '<1 lost');
});

test('a partial row carries its reason through every display, with no lower-bound wording and no shares', () => {
  const reason = 'Some events since the totals\' start have no recorded country; this country\'s share of them is not known';
  const partial = reply({ rows: [row({ tag: 'GER', identity: '7:1', dealt: aircraft(1), taken: aircraft(2), complete: false, partial: reason, causes: [cause(1, aircraft(2))] })], unattributed: ['0', aircraft(3)] });
  const paired = model(partial, PAIRED);
  assert.equal((paired.rows[0] as BarsDataRow).left?.value.value.kind, 'partial');
  assert.ok(!/at least|≥/.test((paired.rows[0] as BarsDataRow).readout!.primary));
  assert.ok(paired.notes?.some(note => note.startsWith('No recorded country:')), 'the witnessed unattributed amount is shown');
  const causes = lossCausesModel(view(partial), AIR);
  const first = causes.rows[0] as BarsDataRow;
  assert.deepEqual([first.segments?.length, first.value?.value.kind], [0, 'partial'], 'no share without a proved denominator');
  const empty = lossCausesModel(view(reply({ rows: [row({ tag: 'GER', identity: '7:1', complete: false, partial: reason })] })), AIR);
  assert.notEqual(empty.empty, 'No aircraft lost yet.', 'an empty partial list is not none');
});

test('an uncertain rank is withheld with its reason, and a requested country not present is one named row', () => {
  const withheld = model(reply({ rows: [], rankMissing: 'Some events have no recorded country and could change this order, so countries cannot be ranked' }));
  assert.deepEqual([withheld.emptyKind, withheld.empty], ['unavailable', 'Some events have no recorded country and could change this order, so countries cannot be ranked']);
  const absent = model(reply({ absent: ['ITA'] }));
  assert.deepEqual(absent.rows.map(item => item.key), ['7:1', '8:2', 'ITA']);
  assert.equal((absent.rows[2] as BarsDataRow).left?.value.value.kind, 'notRecorded');
});

test('loss causes stack the product\'s causes by their recorded names, and an id without a meaning keeps its number and reason', () => {
  const drawn = lossCausesModel(view(reply({ rows: [row({ tag: 'GER', identity: '7:1', taken: aircraft(10), causes: [cause(1, aircraft(6)), cause(7, aircraft(2)), cause(8, aircraft(2))] })] })), AIR);
  assertBarsModelValid(drawn, 500);
  const first = drawn.rows[0] as BarsDataRow;
  assert.deepEqual(first.segments?.map(segment => [segment.key, segment.label]), [['1', 'Aircraft combat'], ['7', 'Cause 7'], ['8', 'Cause 8']]);
  assert.ok(first.tooltip?.items?.some(item => item.missingReason === 'Air loss cause 7 has no proved display meaning'));
});

test('an unread hour is a named state; a group with no member says so', () => {
  const unread = model(reply({ missing: 'Aircraft loss events were not recorded at this hour', rows: [] }));
  assert.deepEqual([unread.empty, unread.emptyKind], ['Aircraft loss events were not recorded at this hour', 'unavailable']);
  assert.equal(model(reply({ rows: [] })).empty, 'No aircraft lost yet.');
  assert.equal(model(reply({ rows: [] }), { ...AIR, group: true }).empty, 'No country of this group at this date.');
});
