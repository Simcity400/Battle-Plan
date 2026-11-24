import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { EquipmentProducedRaceReply } from '../../generated/dashboard';
import { equipmentProducedRaceProjection } from './equipmentProducedRaceProjection.ts';

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

test('equipmentProducedRaceProjection handles default Top unpopulated panel and selected country with missing measure', () => {
  const emptyView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const emptyModel = equipmentProducedRaceProjection.project(emptyView, {
    count: 'units',
    measure: 'day',
    equipmentLabel: 'Infantry Equipment',
    measureLabel: 'Per day',
    noDataMessage: null,
    top: 10,
    all: false,
  });
  assertBarsModelValid(emptyModel, 500);
  assert.equal(emptyModel.rows.length, 0);

  const reply: EquipmentProducedRaceReply = {
    branchId: '1',
    hour: 500,
    firstHour: 480,
    lastHour: 503,
    count: 'units',
    measure: 'day',
    decimals: 0,
    types: ['infantry_equipment'],
    rows: [
      { country: 'GER', tag: 'GER', value: 350, unknown: null },
      { country: 'FRA', tag: 'FRA', value: null, unknown: 'windowIncomplete' },
    ],
    readHour: 480,
    observationHour: 24,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      FRA: { name: 'France', color: '#2563eb' },
    }),
  });
  const model = equipmentProducedRaceProjection.project(view, {
    count: 'units',
    measure: 'day',
    equipmentLabel: 'Infantry Equipment',
    measureLabel: 'Per day',
    noDataMessage: null,
    top: null,
    all: false,
  });
  assertBarsModelValid(model, 500);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows.length, 2);
  assert.equal(rows[0]!.numeric, 350);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});
