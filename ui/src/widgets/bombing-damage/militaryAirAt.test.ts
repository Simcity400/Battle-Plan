import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { MilitaryAirAtReply } from '../../generated/dashboard';
import { militaryAirModel, type MilitaryAirAtOptions } from './militaryAirAt.ts';

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

const AIR: MilitaryAirAtOptions = { family: 'airLosses', scale: 100_000, digits: 5, leftHeader: 'Own losses', rightHeader: 'Enemy aircraft destroyed', unit: 'Aircraft', ratio: true, chosen: null };

const reply = (patch: Partial<MilitaryAirAtReply>): MilitaryAirAtReply => ({
  family: 'airLosses', hour: 500, countedFrom: 0, validUntil: 520, missing: null, rankMissing: null, unattributed: ['0', '0'], warnings: [],
  rows: [
    { tag: 'GER', identity: '7:1', segmentFirstHour: 0, dealt: '445300000', taken: '247400000', complete: true },
    { tag: 'ENG', identity: '8:2', segmentFirstHour: 0, dealt: '143900000', taken: '208500000', complete: true },
  ],
  ...patch,
});

const model = (data: MilitaryAirAtReply, options = AIR) =>
  militaryAirModel(createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', data]]), identity }), options);

test('the product order is drawn as given: own losses left, destroyed right, with the ratio', () => {
  const drawn = model(reply({}));
  assertBarsModelValid(drawn, 500);
  assert.equal(drawn.form, 'two-sided');
  assert.equal(drawn.readoutHeader, 'Ratio');
  const rows = drawn.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.subject), ['GER', 'ENG']);
  assert.deepEqual([formatBarsValueField(rows[0]!.left!.value).text, formatBarsValueField(rows[0]!.right!.value).text], ['2,474', '4,453']);
  assert.equal(rows[0]!.readout?.primary, '1.80 : 1');
  assert.equal(rows[1]!.readout?.primary, '0.69 : 1');
});

test('a row whose events may be missing is partial, at least its value, and has no ratio', () => {
  const rows = model(reply({ rankMissing: 'Some events may be missing', rows: [{ tag: 'GER', identity: '7:1', segmentFirstHour: 0, dealt: '100000', taken: '200000', complete: false }] })).rows as BarsDataRow[];
  assert.equal(rows[0]!.left?.value.value.kind, 'partial');
  assert.equal(rows[0]!.readout, undefined);
});

test('an unread hour and an unranked population are named states, never empty bars', () => {
  const unread = model(reply({ missing: 'Military air events were not read at this hour', rows: [] }));
  assert.equal(unread.emptyKind, 'unavailable');
  const unranked = model(reply({ rankMissing: 'Some events may be missing', rows: [] }));
  assert.equal(unranked.empty, 'Some events may be missing');
});

test('a chosen country that is not present is drawn last as not recorded', () => {
  const rows = model(reply({}), { ...AIR, chosen: ['GER', 'ITA'] }).rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.subject), ['GER', 'ENG', 'ITA']);
  assert.equal(rows[2]!.left?.value.value.kind, 'notRecorded');
});
