import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { ForceAtReply } from '../../generated/dashboard';
import { navySizeRaceProjection, type NavySizeRaceOptions } from './navySizeRaceProjection.ts';

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

const BASE_OPTS: NavySizeRaceOptions = {
  measure: 'strength',
  ownership: 'roster',
  grouping: 'class',
  split: true,
  holders: null,
  membership: null,
  unobserved: [],
  top: 10,
  all: false,
  group: false,
};

test('navy-size race scales fixed-5 hull HP, splits by ship class, and includes maximum hull HP in tooltip', () => {
  const reply: ForceAtReply = {
    parts: [
      { key: 'class:battleship', name: 'Battleship', unresolved: false },
      { key: 'class:destroyer', name: 'Destroyer', unresolved: false },
    ],
    rows: [
      { country: 'ENG', tag: 'ENG', total: 50_000_000, referenceTotal: 60_000_000, missing: null, values: [30_000_000, 20_000_000], partsMissing: null },
      { country: 'JAP', tag: 'JAP', total: null, referenceTotal: null, missing: 'Not recorded', values: [null, null], partsMissing: null },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      JAP: { name: 'Imperial Japan', color: '#f59e0b' },
    }),
  });
  const model = navySizeRaceProjection.project(view, BASE_OPTS);
  assertBarsModelValid(model, 400);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.numeric, 500);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, Math.round(s.share * 100)]), [['Battleship', 60], ['Destroyer', 40]]);
  assert.ok(rows[0]!.tooltip?.items?.some(item => item.label === 'Maximum hull strength (HP)' && item.value === '600'));
  assert.equal(rows[1]!.numeric, null);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});

test('navy-size race under default Top handles unpopulated panel and preserves explicit missing country', () => {
  const unpopulated = createBoundedReadView({ hour: displayedHour(400), panels: new Map() });
  const pending = navySizeRaceProjection.project(unpopulated, { ...BASE_OPTS, measure: 'ships', split: false, top: 10 });
  assert.equal(pending.rows.length, 0);
  assert.equal(pending.empty, undefined);

  const missingExplicit = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map([['main', {
      branchId: '1',
      hour: 400,
      from: 400,
      until: 400,
      refused: 'Ships not recorded at this hour',
      measures: [{ name: 'rosterShips', decimals: 0 }],
      rows: [
        { country: 'ENG', readings: [{ country: 'ENG', hour: 400, run: { firstHour: 400, lastHour: 400, value: 329, valueRaw: '329' }, missing: null }] },
        { country: 'JAP', readings: [{ country: 'JAP', hour: 400, run: null, missing: 'Ships not recorded at this hour' }] },
      ],
    }]]),
    identity: fakeIdentity({
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      JAP: { name: 'Imperial Japan', color: '#f59e0b' },
    }),
  });
  const model = navySizeRaceProjection.project(missingExplicit, {
    ...BASE_OPTS,
    measure: 'ships',
    split: false,
    top: null,
  });
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.value?.value.kind]), [['ENG', 'known'], ['JAP', 'notRecorded']]);
});
