import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { FactionTotalsAtReply, SeriesRankReply } from '../../generated/dashboard';
import { territoryRaceProjection, type TerritoryRaceOptions } from './territoryRaceProjection.ts';

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

function baseOptions(overrides?: Partial<TerritoryRaceOptions>): TerritoryRaceOptions {
  return {
    mode: 'countries',
    measure: 'states',
    rule: 'controlled',
    measureLabel: 'Controlled states',
    scaleUnit: 'States',
    series: { top: 10, all: false, candidates: [] },
    factions: {
      kind: 'territory',
      measure: 'controlledStates',
      measureLabel: 'Controlled states',
      scaleUnit: 'States',
      aggregated: false,
      groupLabel: 'Group',
      top: null,
    },
    ...overrides,
  };
}

test('territoryRaceProjection renders default Top selection with unpopulated panel and preserves missing selected countries', () => {
  const emptyView = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const emptyModel = territoryRaceProjection.project(emptyView, baseOptions());
  assertBarsModelValid(emptyModel, 300);
  assert.equal(emptyModel.rows.length, 0);
  assert.equal(emptyModel.scaleUnit, 'States');

  const reply: SeriesRankReply = {
    hour: 300,
    from: 1,
    until: 400,
    refused: null,
    measures: [{ name: 'controlledStates', decimals: 0 }],
    rows: [
      {
        country: 'GER',
        readings: [
          {
            country: 'GER',
            hour: 300,
            run: { firstHour: 1, lastHour: 400, valueRaw: '42', slot: 1, countryId: 1, startsSegment: true, segmentFirstHour: 1, firstExact: true, lastExact: true },
            drop: null,
            missing: null,
          },
        ],
      },
      {
        country: 'FRA',
        readings: [{ country: 'FRA', hour: 300, run: null, drop: null, missing: 'Not recorded at this hour' }],
      },
    ],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      'country:GER': { name: 'German Reich', color: '#6b7280' },
      'country:FRA': { name: 'France', color: '#2563eb' },
    }),
  });
  const model = territoryRaceProjection.project(view, baseOptions({
    series: { top: null, all: false, candidates: ['GER', 'FRA'] },
  }));
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows.length, 2);
  assert.equal(rows[0]!.numeric, 42);
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});

test('territoryRaceProjection renders faction totals in factions mode', () => {
  const reply: FactionTotalsAtReply = {
    hour: 300,
    kind: 'territory',
    measure: 'controlledStates',
    rows: [
      { id: '100', members: ['GER', 'ITA'], total: 75, missing: null },
      { id: '200', members: ['ENG'], total: null, missing: 'Its roster was not read whole at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      'faction:100': { name: 'Axis', color: '#6b7280' },
      'faction:200': { name: 'Allies', color: '#e11d48' },
      'country:GER': { name: 'German Reich', color: '#6b7280' },
      'country:ITA': { name: 'Italy', color: '#16a34a' },
      'country:ENG': { name: 'United Kingdom', color: '#e11d48' },
    }),
  });
  const model = territoryRaceProjection.project(view, baseOptions({ mode: 'factions' }));
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.label, 'Axis');
  assert.equal(rows[0]!.numeric, 75);
  assert.equal(rows[1]!.label, 'Allies');
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
});
