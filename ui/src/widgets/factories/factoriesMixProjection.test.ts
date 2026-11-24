import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { FactoryMixReply } from '../../generated/dashboard';
import { factoriesMixProjection } from './factoriesMixProjection.ts';

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

test('factoriesMixProjection renders stacked share rows with legend, Factories unit, damaged hatching and refuses unreconciled parts', () => {
  const reply: FactoryMixReply = {
    hour: 400,
    mode: 'built',
    parts: [
      { key: 'military', name: 'Military Factory', damaged: false, group: 'military' },
      { key: 'civilian', name: 'Civilian Factory', damaged: false, group: 'civilian' },
      { key: 'damaged:military', name: 'Damaged Military Factory', damaged: true, group: 'military' },
      { key: 'damaged:civilian', name: 'Damaged Civilian Factory', damaged: true, group: 'civilian' },
    ],
    rows: [
      { country: 'GER', tag: 'GER', total: 100, values: [40, 50, 10, 0], missing: null, partsMissing: null },
      { country: 'SOV', tag: 'SOV', total: 80, values: [], missing: null, partsMissing: 'Damaged factories do not reconcile with built factories at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      SOV: { name: 'Soviet Union', color: '#b91c1c' },
    }),
  });
  const model = factoriesMixProjection.project(view, {
    mode: 'built',
    modeLabel: 'Factories, built',
    top: 10,
    all: false,
  });
  assertBarsModelValid(model, 400);
  assert.equal(model.form, 'share');
  assert.equal(model.scaleUnit, 'Factories');
  assert.deepEqual(model.legend?.map(item => [item.label, Boolean(item.hatched)]), [
    ['Military Factory', false],
    ['Civilian Factory', false],
    ['Damaged Military Factory', true],
  ]);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.readout?.primary, '100');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.share, Boolean(s.hatched)]), [
    ['military', 0.4, false],
    ['civilian', 0.5, false],
    ['damaged:military', 0.1, true],
  ]);
  assert.equal(rows[1]!.segments?.length, 0);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});

test('factoriesMixProjection handles default Top selection with unpopulated panel', () => {
  const view = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const model = factoriesMixProjection.project(view, {
    mode: 'free',
    modeLabel: 'Factories, available',
    top: 10,
    all: false,
  });
  assertBarsModelValid(model, 400);
  assert.equal(model.rows.length, 0);
  assert.equal(model.scaleUnit, 'Factories');
});
