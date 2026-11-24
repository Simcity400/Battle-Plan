import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, type FollowedRows } from '../../dashboard/follow/message.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { assertBarsModelValid, deriveBarsAudit, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { worldTensionRaceProjection, type WorldTensionRaceOptions } from './worldTensionRaceProjection.ts';

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

const BASE_OPTS: WorldTensionRaceOptions = {
  candidates: ['GER', 'JAP', 'ITA'],
  top: 10,
  all: false,
};

test('world-tension race preserves Rust top order, formats fixed-5 percentages, and matches exact audit', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    JAP: { name: 'Imperial Japan', color: '#e5b89b' },
  });

  const followedView = createBoundedReadView({
    hour: displayedHour(500),
    answers: new Map([['main', makeFollowed([['GER', 4525000], ['JAP', 1800000]])]]),
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
      measures: [{ name: 'contribution', decimals: 5 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 500, run: { valueRaw: '4525000', firstHour: 490, lastHour: 510 }, missing: null }] },
        { country: 'JAP', readings: [{ country: 'JAP', hour: 500, run: { valueRaw: '1800000', firstHour: 490, lastHour: 510 }, missing: null }] },
      ],
    }]]),
    identity,
  });

  const followedModel = worldTensionRaceProjection.project(followedView, BASE_OPTS);
  const exactModel = worldTensionRaceProjection.project(exactView, BASE_OPTS);
  assertBarsModelValid(followedModel, 500);
  assertBarsModelValid(exactModel, 500);

  const rows = followedModel.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric, formatBarsValueField(r.value!).text]), [
    ['GER', 45.25, '45.25%'],
    ['JAP', 18, '18%'],
  ]);
  assert.deepEqual(deriveBarsAudit(followedModel), deriveBarsAudit(exactModel));
  assert.throws(() => followedView.who('country', 'GER', displayedHour(501)), FutureDataError);
});

test('world-tension race keeps missing readings as notRecorded with null numeric', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ITA: { name: 'Italy', color: '#4d7c0f' },
  });
  const view = createBoundedReadView({
    hour: displayedHour(200),
    panels: new Map([['main', {
      branchId: '1',
      hour: 200,
      from: 200,
      until: 200,
      refused: 'Threat not recorded',
      measures: [{ name: 'contribution', decimals: 5 }],
      rows: [
        { country: 'GER', readings: [{ country: 'GER', hour: 200, run: { valueRaw: '0', firstHour: 190, lastHour: 210 }, missing: null }] },
        { country: 'ITA', readings: [{ country: 'ITA', hour: 200, run: null, missing: 'Threat not recorded' }] },
      ],
    }]]),
    identity,
  });

  const model = worldTensionRaceProjection.project(view, { candidates: ['ITA', 'GER'], top: null, all: false });
  assertBarsModelValid(model, 200);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.key, 'GER');
  assert.equal(rows[0]!.value?.value.kind, 'zero');
  assert.equal(rows[1]!.key, 'ITA');
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.equal(rows[1]!.numeric, null);
});
