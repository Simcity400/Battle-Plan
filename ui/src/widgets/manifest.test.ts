import assert from 'node:assert/strict';
import test from 'node:test';
import type { WidgetMeta } from '../dashboard/definitions';
import { parseStoredOptions, resolveOptions, setOption, type OptionField, type OptionValue, type WidgetOptions } from '../dashboard/options.ts';
import { addRegion, moveRegion, readRegionChoice, regionParams, removeRegion, writeRegionChoice } from '../families/regions/regionChoice.ts';
import { keyboardMove, regionDropAt, type RegionRowsField } from '../families/regions/regionRows.ts';
import {
  DEFAULT_VIEW,
  declaredViewChoices,
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  viewOwnedFields,
  type FamilyViewEntry,
  type LegacyWidgetEntry,
} from '../families/manifest.ts';
import { bindFamilyManifest, ensureBoundProjection, ensureFamilyProjection, familyProjection, registeredFamilyProjections } from '../families/registry.ts';
import { familyBinding, type FamilyBindingContext, type FamilyBindingIdentity } from '../families/host/adapter.ts';
import type { FamilyTheme } from '../families/contract.ts';
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
import { FAMILY_MANIFESTS, FAMILY_VIEWS } from './familyManifest.ts';
import { LEGACY_WIDGETS } from './legacyManifest.ts';
import { drawnViews } from '../draw/views/drawn.ts';
import {
  activeFamilyView,
  isFamilyWidgetView,
  isPlaybackDatedView,
  resolveWidgetActiveView,
  resolveWidgetScopedOptions,
  resolveWidgetViewSwitch,
  familyViewOptionFields,
  presetStoredOptions,
  widgetOptionFields,
  widgetViewOptions,
} from './manifest.ts';
import { viewSlotProps } from './viewProps.ts';
import type { WidgetSpec } from '../dashboard/model';

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
  assert.equal(FAMILY_VIEWS.length, familyPairs.size, 'each (kind, view) is declared once');
  for (const kind of ['factories', 'air-superiority-region', 'minefields', 'active-research', 'doctrine-mastery', 'decryption']) {
    assert.ok(familyKinds.has(kind), `${kind} must appear in FAMILY_VIEWS`);
  }
});

function otherValue(field: OptionField): OptionValue {
  switch (field.kind) {
    case 'toggle': return !field.default;
    case 'choice': return field.choices.find(choice => choice.value !== field.default)?.value ?? field.default;
    case 'number': return field.default === field.min ? field.max : field.min;
    case 'content-choice':
    case 'content-set': return 'GER';
    case 'regions': return 'r:3a:16';
    case 'table': return 'edited';
  }
}

test('every manifest view reaches its own projection through the binding the host builds, every option its editors write is what the host reads after a reopen, and nothing registers under an undeclared view', async () => {
  const declared = new Set(FAMILY_VIEWS.map(entry => `${entry.family}\u0000${entry.kind}\u0000${normalizeView(entry.view)}`));
  for (const manifest of FAMILY_MANIFESTS) {
    assert.ok(manifest.hostAdapter, `${manifest.family} has a host adapter`);
    for (const entry of manifest.views) {
      const view = normalizeView(entry.view);
      const spec = await entry.loadSpec();
      const declaration = spec.declaration as { readonly kind?: string } | undefined;
      assert.equal(declaration?.kind ?? entry.kind, entry.kind, `${entry.kind}/${view}: the declaration is for its own kind`);
      const ctx: FamilyBindingContext<unknown, unknown> = {
        kind: declaration?.kind ?? entry.kind, view, options: {}, settings: {}, sources: {}, extent: [0, 100], plan: null,
        viewport: { width: 400, height: 300, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null }, theme: {} as FamilyTheme, reducedMotion: false,
        selectedKey: null, focusedKey: null, expanded: {}, audit: false, auditId: undefined, pauseFlush: 0,
      };
      const binding: FamilyBindingIdentity | null = familyBinding(manifest.hostAdapter!, ctx);
      assert.ok(binding, `${entry.kind}/${view}: the ${manifest.family} adapter builds a binding`);
      const lookup = (onReady: () => void): ReturnType<typeof ensureBoundProjection> => ensureBoundProjection(binding, onReady);
      const found = await new Promise<ReturnType<typeof ensureBoundProjection>>(resolve => {
        const first = lookup(() => resolve(lookup(() => {})));
        if (first !== 'loading') resolve(first);
      });
      assert.equal(found, spec.projection, `${entry.kind}/${view}: the worker finds this view's projection`);
      const selected: WidgetOptions = view === DEFAULT_VIEW ? {} : { [entry.viewOption ?? 'view']: view };
      const read = familyViewOptionFields(entry.kind, view);
      for (const [editor, fields] of [['settings panel', resolveWidgetScopedOptions(entry.kind, selected)], ['grid', widgetOptionFields(entry.kind, selected)]] as const) {
        for (const field of fields) {
          if (field.key === (entry.viewOption ?? 'view')) continue;
          const value = otherValue(field);
          if (value === field.default) continue;
          const written = setOption(fields, selected, field.key, value);
          assert.ok(written, `${entry.kind}/${view}: the ${editor} writes ${field.key}`);
          const reopened = parseStoredOptions(JSON.parse(JSON.stringify(written)));
          assert.ok(reopened, `${entry.kind}/${view}: ${field.key} written by the ${editor} survives a save`);
          assert.equal(resolveOptions(read, reopened)[field.key], value, `${entry.kind}/${view}: ${field.key} written by the ${editor} is what the host reads`);
          const handed = viewSlotProps(entry.kind, { widget: { kind: entry.kind, options: reopened } as unknown as WidgetSpec });
          assert.equal(handed.options[field.key], value, `${entry.kind}/${view}: ${field.key} written by the ${editor} is what the view's wrapper, chip and detail window are handed`);
          assert.equal(Object.hasOwn(handed.widget ?? {}, 'options'), false, `${entry.kind}/${view}: the view's wrapper is never handed the stored record`);
        }
      }
    }
  }
  const kinds = new Set(FAMILY_VIEWS.map(entry => entry.kind));
  const stray = registeredFamilyProjections()
    .filter(entry => kinds.has(entry.kind) && !declared.has(`${entry.family}\u0000${entry.kind}\u0000${entry.view}`))
    .map(entry => `${entry.family}/${entry.kind}/${entry.view}`);
  assert.deepEqual(stray, [], 'every registered projection is under a view the manifest declares');
});

test('multi-view widget routes each (kind, view) to its own family or legacy entry with scoped options', () => {
  assert.equal(resolveWidgetActiveView('game-score'), 'table');
  assert.equal(isFamilyWidgetView('game-score'), true);
  assert.equal(activeFamilyView('game-score')?.family, 'table');
  assert.equal(resolveWidgetActiveView('game-score', { view: 'race' }), 'race');
  assert.equal(isFamilyWidgetView('game-score', { view: 'race' }), true);
  assert.equal(activeFamilyView('game-score', { view: 'race' })?.family, 'bars');
  assert.equal(resolveWidgetActiveView('game-score', { view: 'lines' }), 'lines');
  assert.equal(isFamilyWidgetView('game-score', { view: 'lines' }), false);

  assert.equal(activeFamilyView('casualties-between')?.family, 'heat-matrix', 'the view option picks the default');
  assert.equal(activeFamilyView('casualties-between', { view: 'opponents' })?.family, 'bars');
  assert.equal(activeFamilyView('who-spies-on-whom', { view: 'pairs' })?.family, 'bars');
  assert.equal(activeFamilyView('who-spies-on-whom')?.family, 'heat-matrix');

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
  assert.equal(isPlaybackDatedView('map', undefined, metaOf('map')), false);
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

  const settle = async (wakes: readonly number[]) => {
    for (let i = 0; i < 16 && !wakes.length; i++) await Promise.resolve();
  };
  const laneOf = (family: string) => FAMILY_MANIFESTS.find(entry => entry.family === family)!.lane;

  const matrixWakes: number[] = [];
  const matrixView = drawnViews('heat-matrix', wid => matrixWakes.push(wid))!;
  const matrixBinding = { family: 'heat-matrix', kind: 'synthetic-heat-matrix', options: {}, sources: {}, width: 100, height: 100, scrollLeft: 0, scrollTop: 0 };
  const matrixScene = { widget: 7, subs: [], pictures: [], model: matrixBinding };
  assert.equal(matrixView.next!(matrixScene, 50), null, 'a lane whose code is loading locates no change, so the lane paints at its target and walks no history');
  await laneOf('heat-matrix')();
  await settle(matrixWakes);
  assert.deepEqual(matrixWakes, [7], 'the placement that asked is woken once its lane code loads');
  assert.equal(heatMatrixLoads, 0);
  matrixWakes.length = 0;
  assert.equal(matrixView.next!(matrixScene, 50), null, 'a loading projection locates no change, so the lane paints at its target and walks no history');
  await settle(matrixWakes);
  assert.equal(heatMatrixLoads, 1);
  assert.deepEqual(matrixWakes, [7]);
  assert.equal(familyProjection('heat-matrix', 'synthetic-heat-matrix')?.kind, 'synthetic-heat-matrix');

  const lanesWakes: number[] = [];
  const lanesView = drawnViews('heat-lanes', wid => lanesWakes.push(wid))!;
  const lanesBinding = { family: 'heat-lanes', kind: 'synthetic-heat-lanes', options: {}, sources: {}, width: 100, height: 100, scrollTop: 0 };
  const lanesScene = { widget: 9, subs: [], pictures: [], model: lanesBinding };
  assert.equal(lanesView.next!(lanesScene, 50), null);
  await laneOf('heat-lanes')();
  await settle(lanesWakes);
  assert.deepEqual(lanesWakes, [9]);
  lanesWakes.length = 0;
  assert.equal(lanesView.next!(lanesScene, 50), null, 'a loading projection locates no change, so the lane paints at its target and walks no history');
  await settle(lanesWakes);
  assert.equal(heatLanesLoads, 1);
  assert.deepEqual(lanesWakes, [9]);
  assert.equal(familyProjection('heat-lanes', 'synthetic-heat-lanes')?.kind, 'synthetic-heat-lanes');

  assert.deepEqual(
    FAMILY_MANIFESTS.map(entry => entry.family),
    ['bars', 'breakdown', 'card', 'flow', 'heat-lanes', 'heat-matrix', 'meters', 'table', 'time-chart', 'timeline'],
  );
  for (const manifest of FAMILY_MANIFESTS) {
    assert.equal((await manifest.lane()).family, manifest.family, `${manifest.family} lane code loads and names its family`);
  }
});

test('shared data-driven settings controls expose bars-behind, three row heights, columns, view switch, and ordered regions', () => {
  const cols: readonly TableColumnDefaults[] = [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName', 'name'] },
    { key: 'factories', label: 'Factories', as: 'faint', offered: ['plain', 'faint'] },
  ];
  const base: TableBase = { density: 'normal', barsBehind: 'sorted', sort: { column: 'factories', direction: 'descending' } };
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

  const choice = moveRegion(removeRegion(addRegion(addRegion(readRegionChoice(''), 12), 4), 30), 4, 0);
  assert.deepEqual(choice, { picks: [4, 12], hidden: [30] }, 'removing an automatic region hides it');
  assert.deepEqual(readRegionChoice(writeRegionChoice(choice)), choice);
  assert.deepEqual(removeRegion(choice, 12), { picks: [4], hidden: [30] }, 'removing a pick does not hide it');
  assert.deepEqual(addRegion(choice, 30), { picks: [4, 12, 30], hidden: [] }, 'choosing a hidden region picks it');
  assert.deepEqual(regionParams(choice), { pinnedRegions: [4, 12], hiddenRegions: [30] });
  const stored = setOption(fields, undefined, 'regions', writeRegionChoice(choice));
  assert.equal(resolveOptions(fields, stored!).regions, writeRegionChoice(choice));

  const row = (id: number, picked: boolean, top: number) => ({ key: `r${id}`, id, label: `R${id}`, picked, top, height: 26 });
  const shown: RegionRowsField = { rows: [row(4, true, 20), row(12, true, 50), row(7, false, 100), row(9, false, 130)], add: { top: 160, height: 26 }, nameLeft: 8 };
  assert.deepEqual(regionDropAt(shown, shown.rows[1]!, 22), { to: 0, lineY: 20 }, 'a pick dragged above the first pick leads');
  assert.deepEqual(regionDropAt(shown, shown.rows[2]!, 55), { to: 1, lineY: 50 }, 'an automatic row dragged among the picks becomes a pick');
  assert.equal(regionDropAt(shown, shown.rows[3]!, 140), null, 'an automatic row dropped among the automatic rows does nothing');
  assert.deepEqual(keyboardMove(shown.rows[0]!, 1, choice), { picks: [12, 4], hidden: [30] });
  assert.equal(keyboardMove(shown.rows[0]!, -1, choice), null);
  assert.deepEqual(keyboardMove(shown.rows[2]!, -1, choice), { picks: [4, 12, 7], hidden: [30] }, 'Alt+Up on an automatic row picks it');

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
      options: [regionPickerControl(), rowHeightControl()],
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
  assert.deepEqual(scopedOptionFields(undefined, 'bars', entries, legacy).map(field => field.key), ['view', 'regions', 'rowHeight']);
  const tableFields = scopedOptionFields(undefined, 'table', entries, legacy);
  const barsFields = scopedOptionFields(undefined, 'bars', entries, legacy);
  const rowHeightField = tableFields.find(field => field.key === 'rowHeight')!;
  const otherValue = rowHeightField.kind === 'choice' ? rowHeightField.choices.find(choice => choice.value !== rowHeightField.default)!.value : null;
  assert.ok(otherValue);
  const stored = setOption(tableFields, { view: 'table' }, 'rowHeight', otherValue)!;
  assert.equal(resolveOptions(tableFields, stored).rowHeight, otherValue, 'the table view reads the row height it stored');
  assert.equal(resolveOptions(barsFields, { ...stored, view: 'bars' }).rowHeight, rowHeightField.default, 'the bars view keeps its own row height');
  assert.equal(resolveOptions(barsFields, { ...stored, view: 'bars' }).view, 'bars', 'the view switch itself is shared by the kind');
  const picked = setOption(barsFields, { view: 'bars' }, 'regions', 'r:3a:16')!;
  assert.equal(resolveOptions(viewOwnedFields([regionPickerControl()], 'timeline'), picked).regions, 'r:3a:16', 'every view shows the same regions in the same order');
  assert.deepEqual(manifestViewSwitch(undefined, entries, legacy), {
    option: 'view',
    choices: [{ value: 'table', label: 'Table' }, { value: 'bars', label: 'Bars' }],
  });
});

test('two table views of one kind each draw their own projection, and a missing view is refused, never the first view', async () => {
  const projection = (view: string) => ({ family: 'table' as const, kind: 'synthetic-same-family', project: () => ({ columns: [], rows: [], hour: 0, empty: view }) });
  const shares = projection('shares');
  const totals = projection('totals');
  bindFamilyManifest([
    { kind: 'synthetic-same-family', view: 'shares', family: 'table', loadSpec: async () => ({ family: 'table', kind: 'synthetic-same-family', view: 'shares', projection: shares }) },
    { kind: 'synthetic-same-family', view: 'totals', family: 'table', loadSpec: async () => ({ family: 'table', kind: 'synthetic-same-family', view: 'totals', projection: totals }) },
  ]);
  const drain = async (done: () => boolean) => { for (let i = 0; i < 16 && !done(); i++) await Promise.resolve(); };
  let woken = 0;
  assert.equal(ensureFamilyProjection('table', 'synthetic-same-family', 'totals', () => { woken++; }), 'loading');
  await drain(() => woken > 0);
  assert.equal(ensureFamilyProjection('table', 'synthetic-same-family', 'totals', () => {}), totals, 'the totals view draws the totals projection');
  assert.equal(ensureFamilyProjection('table', 'synthetic-same-family', 'shares', () => { woken++; }), 'loading', 'switching to the shares view loads its own projection');
  await drain(() => woken > 1);
  assert.equal(ensureFamilyProjection('table', 'synthetic-same-family', 'shares', () => {}), shares);
  assert.equal(familyProjection('table', 'synthetic-same-family'), undefined, 'a kind with named views has no default view to fall back to');
});

test('a preset is stored through the mapping of the view it opens, so the view reads what the preset chose (R3-S3)', () => {
  const stored = presetStoredOptions('world-tension', { values: 'recorded' });
  assert.equal(resolveWidgetActiveView('world-tension', stored), 'tension');
  assert.equal(widgetViewOptions('world-tension', stored).values, 'recorded', 'the Tension view shows recorded contributions, not its default share');
  const race = presetStoredOptions('army-size', { view: 'race', countFor: 'host' });
  assert.equal(widgetViewOptions('army-size', race).countFor, 'host', 'R3-S2: a race preset counting expeditionary forces for the host is what the race view and its detail read');
  assert.equal(viewSlotProps('army-size', { widget: { kind: 'army-size', options: race } as unknown as WidgetSpec }).options.countFor, 'host');
});
