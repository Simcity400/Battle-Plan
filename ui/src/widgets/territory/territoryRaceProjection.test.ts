import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { FactionTotalsAtReply, SeriesRankReply } from '../../generated/dashboard';
import { territoryRaceDeclaration, territoryRaceProjection, type TerritoryRaceOptions } from './territoryRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string; tag?: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = names[`${kind}:${key}`];
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: hit?.tag ?? key },
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
    group: false,
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

test('territoryRaceProjection sends group to series-rank, keys by slot, preserves zeros and missing readings, and surfaces refusal', () => {
  const emptyView = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map(),
    identity: fakeIdentity({}),
  });
  const emptyModel = territoryRaceProjection.project(emptyView, baseOptions());
  assertBarsModelValid(emptyModel, 300);
  assert.equal(emptyModel.rows.length, 0);
  assert.equal(emptyModel.scaleUnit, 'States');

  const refused: SeriesRankReply = {
    branchId: '1',
    hour: 300,
    from: 300,
    until: 300,
    refused: 'Membership of this group is not recorded at this hour',
    measures: [{ name: 'controlledStates', decimals: 0 }],
    rows: [],
  };
  const refusedModel = territoryRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map([['main', refused]]), identity: fakeIdentity({}) }),
    baseOptions({ group: true }),
  );
  assertBarsModelValid(refusedModel, 300);
  assert.equal(refusedModel.emptyKind, 'unavailable');
  assert.match(refusedModel.empty ?? '', /Membership of this group is not recorded/);

  const reply: SeriesRankReply = {
    branchId: '1',
    hour: 300,
    from: 1,
    until: 400,
    refused: null,
    measures: [{ name: 'controlledStates', decimals: 0 }],
    rows: [
      {
        country: 7,
        slot: 7,
        readings: [{ country: 7, hour: 300, run: { firstHour: 1, lastHour: 400, valueRaw: '42', slot: 7, countryId: 1, startsSegment: true, segmentFirstHour: 1, firstExact: true, lastExact: true }, drop: null, missing: null }],
      },
      {
        country: 9,
        slot: 9,
        readings: [{ country: 9, hour: 300, run: { firstHour: 1, lastHour: 400, valueRaw: '0', slot: 9, countryId: 2, startsSegment: true, segmentFirstHour: 1, firstExact: true, lastExact: true }, drop: null, missing: null }],
      },
      {
        country: 14,
        slot: 14,
        readings: [{ country: 14, hour: 300, run: null, drop: null, missing: 'Not recorded at this hour' }],
      },
    ],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280', tag: 'GER' },
      [`country:${countryKey(9)}`]: { name: 'West Germany', color: '#4b5563', tag: 'GER' },
      [`country:${countryKey(14)}`]: { name: 'France', color: '#2563eb', tag: 'FRA' },
    }),
  });
  const model = territoryRaceProjection.project(view, baseOptions());
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => r.key), [countryKey(7), countryKey(9), countryKey(14)], 'rows keyed by slot');
  assert.equal(rows[0]!.numeric, 42);
  assert.equal(rows[1]!.value?.value.kind, 'zero', 'recorded zero preserved');
  assert.equal(rows[2]!.value?.value.kind, 'notRecorded');

  const plan = territoryRaceDeclaration.sources({
    widget: { id: 'w', kind: 'territory', trackIds: ['t'], countries: { kind: 'group', group: { type: 'war', tag: 'GER' }, membership: 'ever', aggregate: false }, time: { mode: 'follow' }, options: { view: 'race' } },
    track: { id: 't', source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 0, lastHour: 100 },
    options: { view: 'race' },
    settings: {},
    countries: { tags: [], every: [], top: null, all: false, holders: null, membership: null, unobserved: [], reason: null },
  } as never, { following: true, staticSpan: null });
  const read = plan.entries[0]!.read as unknown as { params: Record<string, unknown>; buildExactParams: (base: Record<string, unknown>, hour: number) => Record<string, unknown> | null };
  const exact = read.buildExactParams(read.params, 300);
  assert.deepEqual(exact?.group, { type: 'war', tag: 'GER', membership: 'ever' }, 'group selection sent to series-rank');
});

test('territoryRaceProjection renders faction totals in factions mode using slot member identities', () => {
  const reply: FactionTotalsAtReply = {
    hour: 300,
    kind: 'territory',
    measure: 'controlledStates',
    rows: [
      { id: '100', members: ['GER', 'ITA'], memberCountries: [7, 8], total: 75, missing: null },
      { id: '200', members: ['ENG'], memberCountries: [3], total: null, missing: 'Its roster was not read whole at this hour' },
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
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280', tag: 'GER' },
      [`country:${countryKey(8)}`]: { name: 'Italy', color: '#16a34a', tag: 'ITA' },
      [`country:${countryKey(3)}`]: { name: 'United Kingdom', color: '#e11d48', tag: 'ENG' },
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
