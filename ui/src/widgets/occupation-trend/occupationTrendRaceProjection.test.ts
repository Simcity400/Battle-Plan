import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { occupationTrendRaceProjection, type OccupationTrendRaceOptions } from './occupationTrendRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const BASE_OPTS: OccupationTrendRaceOptions = {
  rank: 'resistance',
  digits: 2,
  top: 10,
  all: false,
  group: false,
};

test('occupation-trend race preserves Rust top order and formats average percent', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ITA: { name: 'Italy', color: '#4d7c0f' },
  });

  const exactView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', {
      branchId: '1',
      hour: 500,
      from: 500,
      until: 500,
      refused: null,
      measures: [{ name: 'resistance', decimals: 5 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 500, run: { valueRaw: '2450000', firstHour: 490, lastHour: 510 }, missing: null }] },
        { country: 'ITA', readings: [{ country: 'ITA', hour: 500, run: { valueRaw: '1200000', firstHour: 490, lastHour: 510 }, missing: null }] },
      ],
    }]]),
    identity,
  });

  const exactModel = occupationTrendRaceProjection.project(exactView, BASE_OPTS);
  assertBarsModelValid(exactModel, 500);

  const rows = exactModel.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric, formatBarsValueField(r.value!).text]), [
    ['GER', 24.5, '24.5%'],
    ['ITA', 12, '12%'],
  ]);
  assert.equal(exactModel.scaleUnit, 'Average resistance');
  assert.throws(() => exactView.who('country', 'GER', displayedHour(501)), FutureDataError);
});

test('occupation-trend race switches label to compliance and keeps missing as notRecorded', () => {
  const identity = fakeIdentity({ GER: { name: 'German Reich', color: '#6b7280' } });
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', {
      branchId: '1',
      hour: 300,
      from: 300,
      until: 300,
      refused: 'Occupation not recorded',
      measures: [{ name: 'compliance', decimals: 5 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 300, run: null, missing: 'Occupation not recorded' }] },
      ],
    }]]),
    identity,
  });

  const model = occupationTrendRaceProjection.project(view, { ...BASE_OPTS, rank: 'compliance', top: null });
  assertBarsModelValid(model, 300);
  assert.equal(model.scaleUnit, 'Average compliance');
  const row = model.rows[0] as BarsDataRow;
  assert.equal(row.value?.value.kind, 'notRecorded');
  assert.equal(row.numeric, null);
});
