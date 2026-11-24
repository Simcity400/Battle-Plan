import assert from 'node:assert/strict';
import test from 'node:test';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, type WidgetOptions } from '../../dashboard/options.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { familyViewOptionFields } from '../manifest.ts';
import '../definitions.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/plan.ts';
import type { AnyTableWidgetDeclaration, DeclaredTableContext } from '../../families/table/declaration.ts';
import { tableHostAdapter } from '../../families/table/hostAdapter.ts';
import type { TableModel, TableProjection } from '../../families/table/model.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, type TableSettings } from '../../families/table/settings.ts';
import type { AgencyComparisonReply } from '../../generated/dashboard';
import { registerGameWindow } from '../../dashboard/gameWindows.ts';
import { TABLE_VIEWS } from './table.ts';

const identity: IdentityProvider = {
  resolve(kind, key, atHour) {
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      tag: { status: 'ready', value: key },
      name: { status: 'ready', value: `${key} name` },
      colour: { status: 'ready', value: '#c0392b' },
      flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000],
      nextChangeAt: Infinity,
    };
  },
};

interface Hosted {
  readonly declaration: AnyTableWidgetDeclaration;
  readonly projection: TableProjection<unknown>;
  readonly options: (stored: WidgetOptions) => Record<string, unknown>;
  readonly settings: (stored: WidgetOptions) => TableSettings;
}

async function hosted(kind: string): Promise<Hosted> {
  const entry = TABLE_VIEWS.find(item => item.kind === kind);
  assert.ok(entry, `${kind} has a table manifest entry`);
  const spec = await entry.loadSpec();
  const declaration = spec.declaration as AnyTableWidgetDeclaration;
  const fields = familyViewOptionFields(kind, entry.view);
  const options = (stored: WidgetOptions) => resolveOptions(fields, stored);
  return {
    declaration,
    projection: spec.projection as TableProjection<unknown>,
    options,
    settings: stored => tableHostAdapter.settings!.resolve(declaration, null, options(stored)),
  };
}

const passingAll = { passing: null, unrecorded: [], bounds: [] };

function context(options: Record<string, unknown>, widget: Partial<WidgetSpec> = {}): DeclaredTableContext {
  const branch = { id: '1', firstHour: 0, lastHour: 1000 };
  return {
    track: { source: { id: 's', revision: 'r' }, branch, axisOffset: 0 },
    branch,
    lastRecordedHour: 1000,
    widget: { countries: { kind: 'all' }, ...widget },
    options,
    following: true,
    aux: undefined,
    countryTag: null,
    countryName: '',
    countries: { all: true, tags: [], holders: null, every: [], top: null, membership: null, membershipRuns: null, unobserved: [], filter: passingAll },
    gateSpan: () => null,
    staticSpan: null,
    settings: undefined,
  } as unknown as DeclaredTableContext;
}

test('stored table settings of the tables A widgets reach the table host through their manifest entries', async () => {
  for (const kind of ['governments-in-exile', 'lend-lease-agreements', 'territory-transfers', 'operatives-roster',
    'industrial-organisations', 'agency-comparison', 'equipment-designs', 'next-elections']) {
    const table = await hosted(kind);
    const defaults = table.settings({});
    const column = defaults.order.find(key => key !== defaults.sort.column)!;
    const changed: TableSettings = {
      ...defaults,
      density: 'compact',
      sort: { column, direction: defaults.sort.direction === 'ascending' ? 'descending' : 'ascending' },
      columns: { ...defaults.columns, [column]: { ...defaults.columns[column]!, width: 123 } },
    };
    const resolved = table.settings({ [TABLE_SETTINGS_KEY]: encodeTableSettings(changed) });
    assert.deepEqual(resolved.sort, changed.sort, `${kind}: the stored sort reaches the table`);
    assert.equal(resolved.density, 'compact', `${kind}: the stored density reaches the table`);
    assert.equal(resolved.columns[column]?.width, 123, `${kind}: the stored column width reaches the table`);
  }
});

test('lend-lease agreements asks the product for what the chosen group receives when the stored direction is Received', async () => {
  const table = await hosted('lend-lease-agreements');
  const widget = { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false } } as Partial<WidgetSpec>;
  const params = (stored: WidgetOptions) => {
    const ctx = context(table.options(stored), widget) as FamilyPlanContext;
    return (table.declaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!.read as unknown as { params: Record<string, unknown> }).params;
  };
  assert.deepEqual(params({ direction: 'received' }), { branchId: '1', countries: [], group: { type: 'faction', id: '42', membership: 'hour' }, only: 'agreements', leaseSide: 'received' },
    'the dated group goes to the product, never every tag that was ever a member');
  assert.equal(params({}).leaseSide, 'sent', 'Sent stays the default');
});

test('next elections and the market send the group and the open tab to the product', async () => {
  const widget = { countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } } as Partial<WidgetSpec>;
  const params = async (kind: string) => {
    const table = await hosted(kind);
    const ctx = context(table.options({}), widget) as FamilyPlanContext;
    return (table.declaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!.read as unknown as { params: Record<string, unknown> }).params;
  };
  assert.deepEqual(await params('next-elections'), { branchId: '1', group: { type: 'humans', membership: 'hour' } });
  const market = TABLE_VIEWS.filter(entry => entry.kind === 'international-market').map(entry => entry.view).sort();
  assert.ok(market.includes('requests'), 'the requests tab has its own view');
  const spec = await TABLE_VIEWS.find(entry => entry.kind === 'international-market' && entry.view === 'requests')!.loadSpec();
  const declaration = spec.declaration as AnyTableWidgetDeclaration;
  const read = declaration.sources(context({}, widget) as FamilyPlanContext, { following: true, staticSpan: null }).entries[0]!.read as unknown as { params: Record<string, unknown> };
  assert.deepEqual(read.params, { branchId: '1', countries: [], group: { type: 'humans', membership: 'hour' }, only: 'requests' }, 'requests are asked as requests, not every kind of terms');
});

test('agency comparison asks the product for the stored top measure and for uncreated agencies', async () => {
  const table = await hosted('agency-comparison');
  const widget = { countries: { kind: 'group', group: { type: 'top', count: 5 } } } as Partial<WidgetSpec>;
  const params = (stored: WidgetOptions) => {
    const ctx = context(table.options(stored), widget) as FamilyPlanContext;
    const read = table.declaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!.read;
    return (read as unknown as { params: { top?: { by: string }; includeUncreated: boolean } }).params;
  };
  assert.equal(params({ topMeasure: 'active' }).top?.by, 'active');
  assert.equal(params({}).top?.by, 'defense', 'counter intelligence stays the default');
  assert.equal(params({ includeUncreated: true }).includeUncreated, true);
  assert.equal(params({}).includeUncreated, false);
});

test('agency comparison opens the agency window by country id, and only for a created agency', async () => {
  const table = await hosted('agency-comparison');
  const agency = (id: number, created: boolean) => ({
    country: { id, tag: `T${id}` }, created, name: created ? `Agency ${id}` : null, nameMissing: created ? null : 'Not created',
    defenseRaw: '0', defenseText: null, defenseValue: null, activeCount: 0, maximum: 0, upgrades: null,
  });
  const reply: AgencyComparisonReply = {
    agencies: [agency(7, true), agency(9, false)], defenseLabel: null, upgradesMissing: null, missing: null, warnings: [],
  };
  const readView = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', reply]]), identity });
  const draw = () => table.projection.project(readView, table.declaration.projectionOptions(context(table.options({ includeUncreated: true }))));
  const view = (model: TableModel) => model.rows.map(row => [row.key, row.cells.view?.value.kind, row.tooltipAction ?? null]);
  assert.deepEqual(view(draw()).map(([key, status]) => [key, status]), [['7', 'notApplicable'], ['9', 'notApplicable']],
    'no View while no agency window is registered');
  registerGameWindow('intelligenceAgency', () => null);
  assert.deepEqual(view(draw()), [['7', 'known', 'Open its agency window'], ['9', 'notApplicable', null]]);
});

test('governments in exile does not report none when an exile with a known other country has an unknown host', async () => {
  const table = await hosted('governments-in-exile');
  const reply = { branchId: '1', view: 'exiles', exiles: [{ id: '9', tag: 'XXX', host: null }], exilesMissing: null, balances: [], current: null, unavailable: [], warnings: [] };
  const readView = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', reply]]), identity });
  const ctx = context(table.options({}));
  const onlyHost = { passing: { HHH: [[0, 1000]] as [number, number][] }, unrecorded: [], bounds: [] };
  const filtered = { ...ctx, countries: { ...ctx.countries, filter: onlyHost } } as DeclaredTableContext;
  const model = table.projection.project(readView, table.declaration.projectionOptions(filtered));
  assert.equal(model.rows.length, 0);
  assert.equal(model.emptyKind, 'unavailable', 'the exile may be hosted by the selected country');
  assert.notEqual(model.empty, 'No governments in exile');
});

test('territory transfers keeps the product order and no rank when its counts are only those observed', async () => {
  const table = await hosted('territory-transfers');
  const country = (key: string, gained: number) => ({ key, tag: key, hour: 10, gained, lost: 0, net: null, netVictoryPoints: null, victoryPointsMissing: 'gap' });
  const reply = { branchId: '1', through: 500, transfers: 3, countries: [country('AAA', 1), country('BBB', 2)], nextChange: null, partial: 'gap', warnings: [] };
  const readView = createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', reply]]), identity });
  const model = table.projection.project(readView, table.declaration.projectionOptions(context(table.options({}))));
  assert.equal(model.populationPartial, 'gap', 'the rows are not table-sorted or ranked as exact');
});
