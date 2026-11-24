import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { FactionTotalsAtReply } from '../../generated/dashboard';
import { factionStrengthRaceProjection } from './factionStrengthRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = names[`${kind}:${key}`];
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

test('factionStrengthRaceProjection handles unpopulated panel, known factions, and missing faction totals', () => {
  const opts = {
    kind: 'country-factories' as const,
    measure: 'free7',
    measureLabel: 'Available factories',
    scaleUnit: 'Factories',
    aggregated: false,
    groupLabel: 'Group',
    top: null,
  };
  const emptyView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const emptyModel = factionStrengthRaceProjection.project(emptyView, opts);
  assertBarsModelValid(emptyModel, 500);
  assert.equal(emptyModel.rows.length, 0);

  const reply: FactionTotalsAtReply = {
    hour: 500,
    kind: 'country-factories',
    measure: 'free7',
    rows: [
      { id: '1', members: ['GER', 'ITA'], total: 240, missing: null },
      { id: '2', members: ['SOV'], total: null, missing: 'A member\'s value (SOV) is not recorded at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      'faction:1': { name: 'Axis', color: '#6b7280' },
      'faction:2': { name: 'Comintern', color: '#b91c1c' },
    }),
  });
  const model = factionStrengthRaceProjection.project(view, opts);
  assertBarsModelValid(model, 500);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.label, 'Axis');
  assert.equal(rows[0]!.numeric, 240);
  assert.equal(rows[1]!.label, 'Comintern');
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});
