import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed, type FollowedRows } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { assertBarsModelValid, deriveBarsAudit, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
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

function makeFollowed(rows: readonly (readonly [string, number | null])[]): FollowedRows {
  const keys = rows.map((_, idx) => BigInt(idx + 1));
  const names = rows.map(([tag]) => tag);
  const values = rows.map(([, val]) => BigInt(val ?? 0));
  const states = rows.map(([, val]) => (val === null ? CELL.missing : CELL.value));
  const reasons = rows.map(([, val]) => (val === null ? 'Not recorded at this hour' : null));
  const parsed = parseFollowed(message(500, [{ id: 1, answer: { keys, columns: 1, values, states, reasons, names } }]));
  return parsed.answers[0]!.answer as FollowedRows;
}

const BASE_OPTS: OccupationTrendRaceOptions = {
  rank: 'resistance',
  digits: 2,
  candidates: ['GER', 'ITA'],
  top: 10,
  all: false,
};

test('occupation-trend race preserves Rust top order, formats average percent, and matches exact audit', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ITA: { name: 'Italy', color: '#4d7c0f' },
  });

  const followedView = createBoundedReadView({
    hour: displayedHour(500),
    answers: new Map([['main', makeFollowed([['GER', 2450000], ['ITA', 1200000]])]]),
    identity,
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

  const followedModel = occupationTrendRaceProjection.project(followedView, BASE_OPTS);
  const exactModel = occupationTrendRaceProjection.project(exactView, BASE_OPTS);
  assertBarsModelValid(followedModel, 500);
  assertBarsModelValid(exactModel, 500);

  const rows = followedModel.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric, formatBarsValueField(r.value!).text]), [
    ['GER', 24.5, '24.5%'],
    ['ITA', 12, '12%'],
  ]);
  assert.equal(followedModel.scaleUnit, 'Average resistance');
  assert.deepEqual(deriveBarsAudit(followedModel), deriveBarsAudit(exactModel));
  assert.throws(() => followedView.who('country', 'GER', displayedHour(501)), FutureDataError);
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

  const model = occupationTrendRaceProjection.project(view, { ...BASE_OPTS, rank: 'compliance', top: null, candidates: ['GER'] });
  assertBarsModelValid(model, 300);
  assert.equal(model.scaleUnit, 'Average compliance');
  const row = model.rows[0] as BarsDataRow;
  assert.equal(row.value?.value.kind, 'notRecorded');
  assert.equal(row.numeric, null);
});
