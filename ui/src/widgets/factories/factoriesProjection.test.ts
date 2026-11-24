import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { factoriesDeclaration, factoriesProjection, type FactoriesProjectionOptions } from './factoriesProjection.ts';

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

type Reading = number | null;

function panelOf(hour: number, rows: readonly (readonly [string, ...Reading[]])[], measures: readonly string[] = ['free7']) {
  return {
    branchId: '1', hour, from: hour, until: hour, refused: null,
    measures: measures.map(name => ({ name, decimals: 0 })),
    rows: rows.map(([country, ...values]) => ({
      country,
      readings: values.map(value => ({
        country, hour, drop: null,
        run: value === null ? null : { valueRaw: String(value), firstHour: hour - 10, lastHour: hour + 10, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true },
        missing: value === null ? 'Factories not recorded' : null,
      })),
    })),
  };
}

const BASE_OPTS: FactoriesProjectionOptions = {
  mode: 'free',
  modeLabel: 'Factories, available',
  split: false,
  parts: [
    { id: 'military', label: 'Military factories', source: 'free1', color: '#3987e5' },
    { id: 'naval', label: 'Naval dockyards', source: 'free2', color: '#d95926' },
    { id: 'civilian', label: 'Civilian factories', source: 'free4', color: '#199e70' },
  ],
  top: 10,
};

test('factories projection preserves Rust top order, uses identity resolver, and matches exact audit', () => {
  const identity = fakeIdentity({
    USA: { name: 'United States', color: '#3b82f6' },
    GER: { name: 'German Reich', color: '#6b7280' },
    SOV: { name: 'Soviet Union', color: '#dc2626' },
  });

  const followedView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', panelOf(500, [['USA', 240], ['GER', 210], ['SOV', 180]])]]),
    identity,
  });
  const exactView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map([['main', panelOf(500, [['USA', 240], ['GER', 210], ['SOV', 180]])]]),
    identity,
  });

  const followedModel = factoriesProjection.project(followedView, BASE_OPTS);
  const exactModel = factoriesProjection.project(exactView, BASE_OPTS);
  assertBarsModelValid(followedModel, 500);
  assertBarsModelValid(exactModel, 500);

  const dataRows = followedModel.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(dataRows.map(r => [r.key, r.label, r.numeric]), [
    ['USA', 'United States', 240],
    ['GER', 'German Reich', 210],
    ['SOV', 'Soviet Union', 180],
  ]);
  assert.equal(followedModel.form, 'ordered');
  assert.equal(followedModel.scaleUnit, 'Factories');
  assert.deepEqual(deriveBarsAudit(followedModel), deriveBarsAudit(exactModel));
});

test('factories projection keeps missing readings as notRecorded with null numeric and keeps the product order with missing last', () => {
  const identity = fakeIdentity({
    GER: { name: 'German Reich', color: '#6b7280' },
    ENG: { name: 'United Kingdom', color: '#e11d48' },
  });
  const view = createBoundedReadView({
    hour: displayedHour(200),
    panels: new Map([['main', panelOf(200, [['GER', 0], ['ENG', null]])]]),
    identity,
  });

  const model = factoriesProjection.project(view, { ...BASE_OPTS, top: null });
  assertBarsModelValid(model, 200);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.equal(rows[0]!.key, 'GER');
  assert.equal(rows[0]!.value?.value.kind, 'zero');
  assert.equal(rows[0]!.numeric, 0);
  assert.equal(rows[1]!.key, 'ENG');
  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.equal(rows[1]!.numeric, null);
});

test('factories projection splits into group segments and rejects reads past displayed hour', () => {
  const identity = fakeIdentity({ GER: { name: 'German Reich', color: '#6b7280' } });
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', panelOf(300, [['GER', 100, 60, 10, 30]], ['free7', 'free1', 'free2', 'free4'])]]),
    identity,
  });

  const model = factoriesProjection.project(view, { ...BASE_OPTS, split: true });
  assertBarsModelValid(model, 300);
  const row = model.rows[0] as BarsDataRow;
  assert.equal(row.segments?.length, 3);
  assert.deepEqual(row.segments?.map(s => [s.key, Math.round(s.share * 100)]), [
    ['military', 60],
    ['naval', 10],
    ['civilian', 30],
  ]);

  assert.throws(() => view.who('country', 'GER', displayedHour(301)), FutureDataError);
});

test('a pinned view with split on asks every part with the total in one rank of the chosen countries', () => {
  const ctx = {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: { split: true, count: 'free' },
    countries: { top: null, all: false, every: ['GER', 'ENG'], tags: ['ENG', 'GER'], membership: null },
  } as unknown as Parameters<typeof factoriesDeclaration.sources>[0];
  const plan = factoriesDeclaration.sources(ctx, { following: false, staticSpan: null });
  assert.deepEqual(plan.entries.map(entry => entry.name), ['main']);
  const main = plan.entries[0]!.read;
  assert.equal(main.shape, 'point');
  const params = main.shape === 'point' ? main.buildExactParams?.(main.params!, 500 as never, 'static') as { top?: number; countries?: string[]; measures?: string[] } : null;
  assert.equal(params?.top, undefined, 'chosen countries are not a top');
  assert.deepEqual(params?.countries, ['ENG', 'GER'], 'the product ranks among the chosen countries');
  assert.ok(params?.measures?.length, 'every part is read in the same reply');
});
