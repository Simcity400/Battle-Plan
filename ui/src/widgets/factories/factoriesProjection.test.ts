import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { boundedEntries } from '../../families/broker.ts';
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

function panelOf(hour: number, rows: readonly (readonly [string, number])[]) {
  return {
    branchId: '1',
    measure: 'free7',
    decimals: 0,
    states: rows.map(([country, value]) => ({ country, hour, run: { valueRaw: String(value), firstHour: hour - 10, lastHour: hour + 10 }, missing: null })),
  };
}

const BASE_OPTS: FactoriesProjectionOptions = {
  mode: 'free',
  modeLabel: 'Factories, available',
  split: false,
  parts: [
    { id: 'military', label: 'Military factories', source: 'part:military', color: '#3987e5' },
    { id: 'naval', label: 'Naval dockyards', source: 'part:naval', color: '#d95926' },
    { id: 'civilian', label: 'Civilian factories', source: 'part:civilian', color: '#199e70' },
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
    panels: new Map([['main', {
      branchId: '1',
      measure: 'free7',
      decimals: 0,
      states: [
        { country: 'USA', hour: 500, run: { valueRaw: '240', firstHour: 490, lastHour: 510 }, missing: null },
        { country: 'GER', hour: 500, run: { valueRaw: '210', firstHour: 490, lastHour: 510 }, missing: null },
        { country: 'SOV', hour: 500, run: { valueRaw: '180', firstHour: 490, lastHour: 510 }, missing: null },
      ],
    }]]),
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
    panels: new Map([['main', {
      branchId: '1',
      measure: 'free7',
      decimals: 0,
      states: [
        { country: 'ENG', hour: 200, run: null, missing: 'Factories not recorded' },
        { country: 'GER', hour: 200, run: { valueRaw: '0', firstHour: 190, lastHour: 210 }, missing: null },
      ],
    }]]),
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
    panels: new Map([
      ['main', panelOf(300, [['GER', 100]])],
      ['part:military', panelOf(300, [['GER', 60]])],
      ['part:naval', panelOf(300, [['GER', 10]])],
      ['part:civilian', panelOf(300, [['GER', 30]])],
    ]),
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

test('a pinned view with split on reads the main and every part source, and the chosen countries are ranked by the product', () => {
  const ctx = {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: { split: true, count: 'free' },
    countries: { top: null, all: false, every: ['GER', 'ENG'], tags: ['ENG', 'GER'], membership: null },
  } as unknown as Parameters<typeof factoriesDeclaration.sources>[0];
  const plan = factoriesDeclaration.sources(ctx, { following: false, staticSpan: null });
  const names = boundedEntries(plan, false, []).map(entry => entry.name);
  assert.ok(names.includes('main'));
  assert.ok(names.some(name => name.startsWith('part:')), `pinned reads ${names.join(', ')}`);
  const main = plan.entries[0]!.read;
  assert.equal(main.shape, 'point');
  const params = main.shape === 'point' ? main.buildExactParams?.(main.params!, 500 as never, 'static') : null;
  assert.equal((params as { top?: number }).top, 2, 'the product ranks among the chosen countries');
});

function partsView(hour: number, panels: Record<string, unknown>) {
  return createBoundedReadView({
    hour: displayedHour(hour),
    panels: new Map(Object.entries(panels)),
    identity: fakeIdentity({ GER: { name: 'German Reich', color: '#6b7280' } }),
  });
}

function missingPanel(hour: number, country: string, reason: string) {
  return { branchId: '1', measure: 'x', decimals: 0, states: [{ country, hour, run: null, missing: reason }] };
}

test('a missing part leaves the rest of the proven total as not recorded, and missing damage is never a damaged share', () => {
  const split = factoriesProjection.project(partsView(300, {
    main: panelOf(300, [['GER', 100]]),
    'part:military': panelOf(300, [['GER', 40]]),
    'part:naval': missingPanel(300, 'GER', 'Dockyards not recorded'),
    'part:civilian': panelOf(300, [['GER', 0]]),
  }), { ...BASE_OPTS, split: true });
  const row = split.rows[0] as BarsDataRow;
  assert.deepEqual(row.segments?.map(s => [s.key, Math.round(s.share * 100), s.value.kind, s.hatched === true]), [
    ['military', 40, 'known', false],
    ['unrecorded', 60, 'notRecorded', true],
  ]);

  const built = { ...BASE_OPTS, mode: 'built' as const, damagedSource: 'damaged' };
  const unknownDamage = factoriesProjection.project(partsView(300, {
    main: panelOf(300, [['GER', 100]]),
    damaged: missingPanel(300, 'GER', 'Damage not recorded'),
  }), built).rows[0] as BarsDataRow;
  assert.equal(unknownDamage.segments, undefined, 'the whole bar stays one known total');
  assert.deepEqual(unknownDamage.tooltip?.items?.find(item => item.label === 'Damaged'), { label: 'Damaged', missingReason: 'Damage not recorded' });

  const partDamage = factoriesProjection.project(partsView(300, {
    main: panelOf(300, [['GER', 100]]),
    'part:military': panelOf(300, [['GER', 60]]),
    'part:naval': panelOf(300, [['GER', 10]]),
    'part:civilian': panelOf(300, [['GER', 30]]),
    'damaged:military': panelOf(300, [['GER', 6]]),
    'damaged:naval': missingPanel(300, 'GER', 'Damage not recorded'),
    'damaged:civilian': panelOf(300, [['GER', 0]]),
  }), {
    ...built,
    split: true,
    parts: BASE_OPTS.parts.map(part => ({ ...part, damagedSource: `damaged:${part.id}` })),
  }).rows[0] as BarsDataRow;
  assert.deepEqual(partDamage.segments?.map(s => [s.key, Math.round(s.share * 100)]), [
    ['military', 54], ['damaged:military', 6], ['naval', 10], ['civilian', 30],
  ]);
});

test('each part panel is read and indexed once per picture, not once per country and part', () => {
  const countries = Array.from({ length: 40 }, (_, at) => [`C${at}`, 100 - at] as const);
  const panels: Record<string, unknown> = { main: panelOf(300, countries) };
  for (const part of BASE_OPTS.parts) panels[part.source] = panelOf(300, countries.map(([tag]) => [tag, 10] as const));
  const view = partsView(300, panels);
  const reads = new Map<string, number>();
  const counted = { ...view, panel: (name: string) => { reads.set(name, (reads.get(name) ?? 0) + 1); return view.panel(name); } };
  factoriesProjection.project(counted as typeof view, { ...BASE_OPTS, split: true });
  assert.deepEqual([...reads.values()].every(count => count === 1), true, JSON.stringify([...reads]));
});

test('a factory type keeps its colour when a type before it is deselected', () => {
  const ctx = (types: string) => ({
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: { split: true, count: 'free', types },
    countries: { top: 5, all: false, every: [], tags: [], membership: null, holders: null, unobserved: [] },
  }) as unknown as Parameters<typeof factoriesDeclaration.projectionOptions>[0];
  const colourOf = (types: string, id: string) => factoriesDeclaration.projectionOptions(ctx(types)).parts.find(part => part.id === id)!.color;
  assert.equal(colourOf('naval civilian', 'civilian'), colourOf('', 'civilian'));
  assert.equal(colourOf('civilian', 'civilian'), colourOf('', 'civilian'));
});

test('a group with no member at the displayed hour asks nothing and says so', () => {
  const ctx = {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1' },
    options: { count: 'free' },
    countries: { top: null, all: false, every: [], tags: ['ITA'], membership: { ITA: [[600, 900]] } },
  } as unknown as Parameters<typeof factoriesDeclaration.sources>[0];
  const main = factoriesDeclaration.sources(ctx, { following: false, staticSpan: null }).entries[0]!.read;
  assert.equal(main.shape, 'point');
  if (main.shape !== 'point') return;
  assert.equal(main.buildExactParams?.(main.params!, 500 as never, 'static'), null);
  assert.deepEqual((main.buildExactParams?.(main.params!, 700 as never, 'static') as { reads: unknown }).reads, [{ country: 'ITA', hour: 700 }]);
  const model = factoriesProjection.project(partsView(500, {}), { ...BASE_OPTS, top: null, membership: { ITA: [[600, 900]] } });
  assert.equal(model.emptyKind, 'empty');
  assert.equal(model.empty, 'No country of this group at this date.');
});
