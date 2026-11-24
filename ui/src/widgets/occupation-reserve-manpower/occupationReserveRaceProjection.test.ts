import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { OccupationReserveReply } from '../../generated/dashboard';
import { occupationReserveRaceProjection } from './occupationReserveRaceProjection.ts';

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

test('occupation-reserve-manpower race preserves Rust ranking, builds source-country segments, and separates negative entries in tooltip', () => {
  const reply: OccupationReserveReply = {
    holders: [
      {
        country: { id: 2, tag: 'GER' },
        admitted: true,
        total: 500,
        entries: [
          { ordinal: 0, source: { id: 8, tag: 'ITA' }, amount: -50 },
          { ordinal: 1, source: { id: 9, tag: 'HUN' }, amount: 300 },
          { ordinal: 2, source: { id: 9, tag: 'HUN' }, amount: 200 },
        ],
      },
      {
        country: { id: 3, tag: 'ENG' },
        admitted: true,
        total: 250,
        entries: [
          { ordinal: 0, source: { id: 10, tag: null }, amount: 250 },
        ],
      },
    ],
    readHour: 240,
    unnamed: 1,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(240),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      ITA: { name: 'Italy', color: '#22c55e' },
      HUN: { name: 'Hungary', color: '#f59e0b' },
    }),
  });
  const model = occupationReserveRaceProjection.project(view, { top: 10, all: false });
  assertBarsModelValid(model, 240);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric]), [['GER', 500], ['ENG', 250]]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.share]), [['HUN', 1]]);
  assert.ok(rows[0]!.tooltip?.items?.some(item => item.label === 'Negative entries'));
  assert.ok(model.warnings?.some(w => w.includes('1 holders')));
});

test('occupation-reserve-manpower surfaces missing refusal as unavailable and handles unpopulated default Top cleanly', () => {
  const unpopulated = createBoundedReadView({ hour: displayedHour(240), panels: new Map() });
  const pending = occupationReserveRaceProjection.project(unpopulated, { top: 10, all: false });
  assert.equal(pending.rows.length, 0);
  assert.equal(pending.empty, undefined);

  const missing: OccupationReserveReply = {
    holders: [],
    readHour: null,
    unnamed: 0,
    missing: 'Occupation reserve was not recorded at this hour',
    warnings: [],
  };
  const refused = occupationReserveRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(240), panels: new Map([['main', missing]]) }),
    { top: 10, all: false },
  );
  assert.equal(refused.emptyKind, 'unavailable');
  assert.equal(refused.empty, missing.missing);
});
