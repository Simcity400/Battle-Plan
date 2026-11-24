import assert from 'node:assert/strict';
import test from 'node:test';
import type { WidgetMeta } from '../dashboard/definitions';
import { addOrderedToken, contentSet, moveOrderedToken, removeOrderedToken, resolveOptions, setOption } from '../dashboard/options.ts';
import {
  DEFAULT_VIEW,
  declaredViewChoices,
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  type FamilyViewEntry,
  type LegacyWidgetEntry,
} from '../families/manifest.ts';
import { bindFamilyManifest, ensureFamilyProjection, familyProjection } from '../families/registry.ts';
import {
  BARS_BEHIND_CHOICES,
  ROW_HEIGHT_CHOICES,
  barsBehindControl,
  barsBehindOf,
  columnsControl,
  regionPickerControl,
  rowHeightControl,
  rowHeightOf,
  viewSwitchControl,
} from '../families/settingsControls.ts';
import { encodeTableSettings, ROW_HEIGHT, tableDefaults, tableSettingsOf, updateTableSettings, type TableBase, type TableColumnDefaults } from '../families/table/settings.ts';
import { widgetMetas } from './definitions.ts';
import { FAMILY_MANIFESTS, FAMILY_VIEWS, familyLaneView } from './familyManifest.ts';
import { LEGACY_WIDGETS } from './legacyManifest.ts';
import { drawnViews } from '../draw/views/drawn.ts';
import {
  activeFamilyView,
  isFamilyWidgetView,
  isPlaybackDatedView,
  resolveWidgetActiveView,
  resolveWidgetScopedOptions,
  resolveWidgetViewSwitch,
} from './manifest.ts';

test('manifest accounts for every widget kind once across family views and legacy entries', () => {
  const familyKinds = new Set(FAMILY_VIEWS.map(entry => entry.kind));
  const legacyKinds = new Set(LEGACY_WIDGETS.map(entry => entry.kind));
  for (const meta of widgetMetas) {
    assert.ok(familyKinds.has(meta.kind) || legacyKinds.has(meta.kind), `${meta.kind} must appear in FAMILY_VIEWS or LEGACY_WIDGETS`);
  }
  const familyPairs = new Set(FAMILY_VIEWS.map(entry => `${entry.kind}:${normalizeView(entry.view)}`));
  for (const entry of LEGACY_WIDGETS) {
    const pair = `${entry.kind}:${normalizeView(entry.view)}`;
    assert.equal(familyPairs.has(pair), false, `${pair} must not appear in both lists`);
  }
  assert.ok(FAMILY_VIEWS.length >= 10);
  for (const kind of ['factories', 'air-superiority-region', 'minefields', 'active-research', 'doctrine-mastery', 'decryption']) {
    assert.ok(familyKinds.has(kind), `${kind} must appear in FAMILY_VIEWS`);
  }
});

test('multi-view widget routes each (kind, view) to its own family or legacy entry with scoped options', () => {
  assert.equal(resolveWidgetActiveView('game-score'), 'table');
  assert.equal(isFamilyWidgetView('game-score'), true);
  assert.equal(activeFamilyView('game-score')?.family, 'table');
  assert.equal(resolveWidgetActiveView('game-score', { view: 'race' }), 'race');
  assert.equal(isFamilyWidgetView('game-score', { view: 'race' }), false);
  assert.equal(resolveWidgetActiveView('game-score', { view: 'lines' }), 'lines');
  assert.equal(isFamilyWidgetView('game-score', { view: 'lines' }), false);

  const tableKeys = resolveWidgetScopedOptions('game-score', { view: 'table' }).map(field => field.key);
  assert.ok(tableKeys.includes('view'));
  assert.ok(tableKeys.includes('factions'));
  assert.equal(tableKeys.includes('endLabels'), false);
  assert.equal(tableKeys.includes('split'), false);

  const linesKeys = resolveWidgetScopedOptions('game-score', { view: 'lines' }).map(field => field.key);
  assert.ok(linesKeys.includes('endLabels'));

  const viewSwitch = resolveWidgetViewSwitch('game-score');
  assert.equal(viewSwitch?.option, 'view');
  assert.deepEqual(viewSwitch?.choices, [
    { value: 'table', label: 'Table' },
    { value: 'race', label: 'Race' },
    { value: 'lines', label: 'Lines over time' },
  ]);
});

test('playback placeholder gate uses the manifest as its single switch', () => {
  const metaOf = (kind: string): WidgetMeta | undefined => widgetMetas.find(item => item.kind === kind);
  for (const entry of FAMILY_VIEWS) {
    const meta = metaOf(entry.kind);
    assert.equal(meta?.playsBackDated, undefined, `${entry.kind} drops playsBackDated once ported`);
    const opts = entry.view && entry.view !== DEFAULT_VIEW ? { [entry.viewOption ?? 'view']: entry.view } : undefined;
    assert.equal(isPlaybackDatedView(entry.kind, opts, meta), true, `(${entry.kind}, ${entry.view ?? DEFAULT_VIEW}) plays back live`);
  }
  assert.equal(isPlaybackDatedView('research', undefined, metaOf('research')), true);
  assert.equal(isPlaybackDatedView('casualties', undefined, metaOf('casualties')), false);
  assert.equal(isPlaybackDatedView('game-score', { view: 'table' }), true);
  assert.equal(isPlaybackDatedView('game-score', { view: 'race' }), true);
});

test('worker projection registry loads table and bars projections lazily on demand and wakes waiters', async () => {
  let tableLoads = 0;
  let barsLoads = 0;
  const fakeTableEntry: FamilyViewEntry = {
    kind: 'synthetic-lazy-table',
    view: 'summary',
    family: 'table',
    loadSpec: async () => {
      tableLoads++;
      return {
        family: 'table',
        kind: 'synthetic-lazy-table',
        view: 'summary',
        projection: { family: 'table', kind: 'synthetic-lazy-table', project: () => ({ columns: [], rows: [], hour: 0 }) },
      };
    },
  };
  const fakeBarsEntry: FamilyViewEntry = {
    kind: 'synthetic-lazy-bars',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: async () => {
      barsLoads++;
      return {
        family: 'bars',
        kind: 'synthetic-lazy-bars',
        view: DEFAULT_VIEW,
        projection: { family: 'bars', kind: 'synthetic-lazy-bars', project: () => ({ form: 'single', rows: [], hour: 0 }) },
      };
    },
  };
  bindFamilyManifest([fakeTableEntry, fakeBarsEntry]);

  assert.equal(ensureFamilyProjection('table', 'nonexistent-kind', DEFAULT_VIEW, () => {}), 'unknown');

  let tableWakes = 0;
  const tableDone = new Promise<void>(resolve => {
    const first = ensureFamilyProjection('table', 'synthetic-lazy-table', 'summary', () => { tableWakes++; resolve(); });
    const second = ensureFamilyProjection('table', 'synthetic-lazy-table', 'summary', () => { tableWakes++; });
    assert.equal(first, 'loading');
    assert.equal(second, 'loading');
  });
  await tableDone;
  assert.equal(tableLoads, 1);
  assert.equal(tableWakes, 2);
  assert.equal(familyProjection('table', 'synthetic-lazy-table', 'summary')?.kind, 'synthetic-lazy-table');

  let barsWakes = 0;
  const barsDone = new Promise<void>(resolve => {
    assert.equal(ensureFamilyProjection('bars', 'synthetic-lazy-bars', DEFAULT_VIEW, () => { barsWakes++; resolve(); }), 'loading');
  });
  await barsDone;
  assert.equal(barsLoads, 1);
  assert.equal(barsWakes, 1);
  assert.equal(familyProjection('bars', 'synthetic-lazy-bars')?.kind, 'synthetic-lazy-bars');

  let timeChartWakes = 0;
  const pp = ensureFamilyProjection('time-chart', 'political-power', DEFAULT_VIEW, () => { timeChartWakes++; });
  if (pp === 'loading') {
    await new Promise<void>(resolve => {
      ensureFamilyProjection('time-chart', 'political-power', DEFAULT_VIEW, resolve);
    });
    assert.ok(timeChartWakes >= 1);
  }
  assert.equal(familyProjection('time-chart', 'political-power')?.kind, 'political-power');

  let heatMatrixLoads = 0;
  let heatLanesLoads = 0;
  bindFamilyManifest([
    {
      kind: 'synthetic-heat-matrix',
      view: DEFAULT_VIEW,
      family: 'heat-matrix',
      loadSpec: async () => {
        heatMatrixLoads++;
        return {
          family: 'heat-matrix',
          kind: 'synthetic-heat-matrix',
          view: DEFAULT_VIEW,
          projection: { family: 'heat-matrix', kind: 'synthetic-heat-matrix', project: () => ({ rows: [], columns: [], cells: [], hour: 0 }) },
        };
      },
    },
    {
      kind: 'synthetic-heat-lanes',
      view: DEFAULT_VIEW,
      family: 'heat-lanes',
      loadSpec: async () => {
        heatLanesLoads++;
        return {
          family: 'heat-lanes',
          kind: 'synthetic-heat-lanes',
          view: DEFAULT_VIEW,
          projection: { family: 'heat-lanes', kind: 'synthetic-heat-lanes', project: () => ({ rows: [], hour: 0 }) },
        };
      },
    },
  ]);

  const matrixWakes: number[] = [];
  const matrixView = drawnViews('heat-matrix', wid => matrixWakes.push(wid))!;
  const matrixBinding = { family: 'heat-matrix', kind: 'synthetic-heat-matrix', options: {}, sources: {}, width: 100, height: 100, scrollLeft: 0, scrollTop: 0 };
  assert.equal(matrixView.next!({ widget: 7, subs: [], pictures: [], model: matrixBinding }, 50), 51);
  for (let i = 0; i < 16 && !matrixWakes.length; i++) await Promise.resolve();
  assert.equal(heatMatrixLoads, 1);
  assert.deepEqual(matrixWakes, [7]);
  assert.equal(familyProjection('heat-matrix', 'synthetic-heat-matrix')?.kind, 'synthetic-heat-matrix');

  const lanesWakes: number[] = [];
  const lanesView = drawnViews('heat-lanes', wid => lanesWakes.push(wid))!;
  const lanesBinding = { family: 'heat-lanes', kind: 'synthetic-heat-lanes', options: {}, sources: {}, width: 100, height: 100, scrollTop: 0 };
  assert.equal(lanesView.next!({ widget: 9, subs: [], pictures: [], model: lanesBinding }, 50), 51);
  for (let i = 0; i < 16 && !lanesWakes.length; i++) await Promise.resolve();
  assert.equal(heatLanesLoads, 1);
  assert.deepEqual(lanesWakes, [9]);
  assert.equal(familyProjection('heat-lanes', 'synthetic-heat-lanes')?.kind, 'synthetic-heat-lanes');

  assert.deepEqual(
    FAMILY_MANIFESTS.map(entry => entry.family),
    ['bars', 'heat-lanes', 'heat-matrix', 'meters', 'table', 'time-chart'],
  );
  for (const family of ['bars', 'heat-lanes', 'heat-matrix', 'meters', 'table', 'time-chart']) {
    assert.ok(familyLaneView(family, () => {}), `familyLaneView resolves ${family}`);
  }
});

test('shared data-driven settings controls expose bars-behind, three row heights, columns, view switch, and ordered regions', () => {
  const cols: readonly TableColumnDefaults[] = [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName', 'name'] },
    { key: 'factories', label: 'Factories', as: 'faint', offered: ['plain', 'faint'] },
  ];
  const base: TableBase = { density: 'normal', barsBehind: 'sorted', overflow: 'tooltip', sort: { column: 'factories', direction: 'descending' } };
  const fields = [
    viewSwitchControl([{ value: 'rows', label: 'Rows' }, { value: 'bars', label: 'Bars' }], 'rows'),
    rowHeightControl('normal'),
    barsBehindControl('sorted'),
    columnsControl(cols, base),
    regionPickerControl(),
  ];

  const defaults = resolveOptions(fields, undefined);
  assert.equal(defaults.view, 'rows');
  assert.equal(defaults.rowHeight, 'normal');
  assert.equal(defaults.barsBehind, 'sorted');
  assert.equal(defaults.regions, '');
  assert.equal(ROW_HEIGHT_CHOICES.length, 3);
  assert.equal(BARS_BEHIND_CHOICES.length, 3);
  assert.deepEqual(ROW_HEIGHT, { compact: 28, normal: 36, comfortable: 44 });

  assert.equal(rowHeightOf('compact'), 'compact');
  assert.equal(rowHeightOf('normal'), 'normal');
  assert.equal(rowHeightOf('comfortable'), 'comfortable');
  assert.equal(rowHeightOf('bogus'), 'normal');
  assert.equal(barsBehindOf('off'), 'off');
  assert.equal(barsBehindOf('sorted'), 'sorted');
  assert.equal(barsBehindOf('all'), 'all');
  assert.equal(barsBehindOf('bogus'), 'all');
  assert.deepEqual(BARS_BEHIND_CHOICES.map(c => c.label), ['Every column', 'Sorted column', 'Off']);
  assert.deepEqual(ROW_HEIGHT_CHOICES.map(c => c.label), ['Compact', 'Default', 'Comfortable']);

  const added1 = addOrderedToken('', 'reg_1');
  const added2 = addOrderedToken(added1!, 'reg_2');
  const added3 = addOrderedToken(added2!, 'reg_3');
  assert.deepEqual(contentSet(added3!), ['reg_1', 'reg_2', 'reg_3']);
  const moved = moveOrderedToken(added3!, 2, -1);
  assert.deepEqual(contentSet(moved), ['reg_1', 'reg_3', 'reg_2']);
  const removed = removeOrderedToken(moved, 'reg_3');
  assert.deepEqual(contentSet(removed), ['reg_1', 'reg_2']);
  const stored = setOption(fields, undefined, 'regions', removed);
  assert.deepEqual(stored, { regions: 'reg_1 reg_2' });
  assert.equal(resolveOptions(fields, stored!).regions, 'reg_1 reg_2');

  const tblDefaults = tableDefaults(cols, base);
  assert.equal(tblDefaults.density, 'normal');
  assert.equal(tblDefaults.barsBehind, 'sorted');
  const updated = updateTableSettings(tblDefaults, next => {
    next.density = 'comfortable';
    next.barsBehind = 'all';
  });
  const roundTripped = tableSettingsOf(encodeTableSettings(updated), cols, base);
  assert.equal(roundTripped.density, 'comfortable');
  assert.equal(roundTripped.barsBehind, 'all');
});

test('scopedOptionFields and manifestViewSwitch support a second family view on the same kind', () => {
  const entries: readonly FamilyViewEntry[] = [
    {
      kind: 'synthetic-dual',
      view: 'table',
      label: 'Table',
      family: 'table',
      options: [rowHeightControl(), barsBehindControl()],
      loadSpec: async () => ({ family: 'table', kind: 'synthetic-dual', view: 'table', projection: { family: 'table', kind: 'synthetic-dual', project: () => ({}) } }),
    },
    {
      kind: 'synthetic-dual',
      view: 'bars',
      label: 'Bars',
      family: 'bars',
      options: [regionPickerControl()],
      loadSpec: async () => ({ family: 'bars', kind: 'synthetic-dual', view: 'bars', projection: { family: 'bars', kind: 'synthetic-dual', project: () => ({}) } }),
    },
  ];
  const legacy: readonly LegacyWidgetEntry[] = [];
  const choices = declaredViewChoices(undefined, entries, legacy);
  assert.deepEqual(choices.map(item => [item.value, item.label, item.family]), [
    ['table', 'Table', 'table'],
    ['bars', 'Bars', 'bars'],
  ]);
  assert.equal(resolveActiveView(undefined, undefined, entries, legacy), 'table');
  assert.equal(resolveActiveView({ view: 'bars' }, undefined, entries, legacy), 'bars');
  assert.deepEqual(scopedOptionFields(undefined, 'table', entries, legacy).map(field => field.key), ['view', 'rowHeight', 'barsBehind']);
  assert.deepEqual(scopedOptionFields(undefined, 'bars', entries, legacy).map(field => field.key), ['view', 'regions']);
  assert.deepEqual(manifestViewSwitch(undefined, entries, legacy), {
    option: 'view',
    choices: [{ value: 'table', label: 'Table' }, { value: 'bars', label: 'Bars' }],
  });
});
