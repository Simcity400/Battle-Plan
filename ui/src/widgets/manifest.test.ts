import assert from 'node:assert/strict';
import test from 'node:test';
import { parseStoredOptions, resolveOptions, setOption, type OptionField, type OptionValue, type WidgetOptions } from '../dashboard/options.ts';
import { addRegion, moveRegion, readRegionChoice, regionParams, removeRegion, writeRegionChoice } from '../families/regions/regionChoice.ts';
import { chooserFor, keyboardMove, regionChooserGroups, regionDropAt, savedRegionName, type RegionRowsField } from '../families/regions/regionRows.ts';
import {
  DEFAULT_VIEW,
  declaredViewChoices,
  manifestViewSwitch,
  normalizeView,
  resolveActiveView,
  scopedOptionFields,
  viewOwnedFields,
  type FamilyViewEntry,
} from '../families/manifest.ts';
import { bindFamilyManifest, ensureBoundProjection, ensureFamilyProjection, registeredFamilyProjections } from '../families/registry.ts';
import { familyBinding, type FamilyBindingContext, type FamilyBindingIdentity } from '../families/host/adapter.ts';
import type { FamilyTheme } from '../families/contract.ts';
import {
  BARS_BEHIND_CHOICES,
  ROW_HEIGHT_CHOICES,
  columnsControl,
  regionPickerControl,
  viewSwitchControl,
} from '../families/settingsControls.ts';
import { encodeTableSettings, ROW_HEIGHT, tableDefaults, tableSettingsOf, updateTableSettings, type TableBase, type TableColumnDefaults } from '../families/table/settings.ts';
import { widgetMetas } from './definitions.ts';
import { FAMILY_MANIFESTS, FAMILY_VIEWS } from './familyManifest.ts';
import { drawnViews } from '../draw/views/drawn.ts';
import {
  NATIVE_WIDGETS,
  resolveWidgetActiveView,
  resolveWidgetScopedOptions,
  resolveWidgetViewSwitch,
  familyViewOptionFields,
  presetWidget,
  widgetOptionFields,
  widgetViewOptions,
} from './manifest.ts';
import { viewSlotProps } from './viewProps.ts';
import type { WidgetSpec } from '../dashboard/model';

test('manifest accounts for every widget kind across family and native entries', () => {
  const familyKinds = new Set(FAMILY_VIEWS.map(entry => entry.kind));
  assert.deepEqual([...new Set([...FAMILY_VIEWS, ...NATIVE_WIDGETS].map(entry => entry.kind))].sort(), widgetMetas.map(meta => meta.kind).sort());
  assert.ok(NATIVE_WIDGETS.some(entry => entry.kind === 'map'));
  for (const { kind } of NATIVE_WIDGETS) {
    assert.ok(!familyKinds.has(kind), `${kind} has its own native renderer`);
  }
  const familyPairs = new Set(FAMILY_VIEWS.map(entry => `${entry.kind}:${normalizeView(entry.view)}`));
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

test('multi-view widget routes each (kind, view) to its own family entry with scoped options', () => {
  const familyFor = (kind: string, options?: WidgetOptions) => {
    const view = resolveWidgetActiveView(kind, options);
    return FAMILY_VIEWS.find(entry => entry.kind === kind && normalizeView(entry.view) === view);
  };
  assert.equal(resolveWidgetActiveView('game-score'), 'table');
  assert.equal(familyFor('game-score')?.family, 'table');
  assert.equal(resolveWidgetActiveView('game-score', { view: 'race' }), 'race');
  assert.equal(familyFor('game-score', { view: 'race' })?.family, 'bars');
  assert.equal(resolveWidgetActiveView('game-score', { view: 'lines' }), 'lines');
  assert.equal(familyFor('game-score', { view: 'lines' })?.family, 'time-chart');

  assert.equal(familyFor('casualties-between')?.family, 'heat-matrix', 'the view option picks the default');
  assert.equal(familyFor('casualties-between', { view: 'opponents' })?.family, 'bars');
  assert.equal(familyFor('who-spies-on-whom', { view: 'pairs' })?.family, 'bars');
  assert.equal(familyFor('who-spies-on-whom')?.family, 'heat-matrix');

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
  const loadedKind = (family: string, kind: string, view?: string) => {
    const found = ensureFamilyProjection(family as never, kind, view, () => {});
    return typeof found === 'object' ? found.kind : undefined;
  };
  await tableDone;
  assert.equal(tableLoads, 1);
  assert.equal(tableWakes, 2);
  assert.equal(loadedKind('table', 'synthetic-lazy-table', 'summary'), 'synthetic-lazy-table');

  let barsWakes = 0;
  const barsDone = new Promise<void>(resolve => {
    assert.equal(ensureFamilyProjection('bars', 'synthetic-lazy-bars', DEFAULT_VIEW, () => { barsWakes++; resolve(); }), 'loading');
  });
  await barsDone;
  assert.equal(barsLoads, 1);
  assert.equal(barsWakes, 1);
  assert.equal(loadedKind('bars', 'synthetic-lazy-bars'), 'synthetic-lazy-bars');

  let timeChartWakes = 0;
  const pp = ensureFamilyProjection('time-chart', 'political-power', DEFAULT_VIEW, () => { timeChartWakes++; });
  if (pp === 'loading') {
    await new Promise<void>(resolve => {
      ensureFamilyProjection('time-chart', 'political-power', DEFAULT_VIEW, resolve);
    });
    assert.ok(timeChartWakes >= 1);
  }
  assert.equal(loadedKind('time-chart', 'political-power'), 'political-power');

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
  assert.equal(loadedKind('heat-matrix', 'synthetic-heat-matrix'), 'synthetic-heat-matrix');

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
  assert.equal(loadedKind('heat-lanes', 'synthetic-heat-lanes'), 'synthetic-heat-lanes');

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
    columnsControl(cols, base),
    regionPickerControl(),
  ];

  const defaults = resolveOptions(fields, undefined);
  assert.equal(defaults.view, 'rows');
  assert.equal(defaults.regions, '');
  assert.equal(ROW_HEIGHT_CHOICES.length, 3);
  assert.equal(BARS_BEHIND_CHOICES.length, 3);
  assert.deepEqual(ROW_HEIGHT, { compact: 28, normal: 36, comfortable: 44 });
  assert.deepEqual(BARS_BEHIND_CHOICES.map(c => c.label), ['Every column', 'Sorted column', 'Off']);
  assert.deepEqual(ROW_HEIGHT_CHOICES.map(c => c.label), ['Compact', 'Default', 'Comfortable']);

  const choice = moveRegion(removeRegion(addRegion(addRegion(readRegionChoice(''), 12), 4), 30), 4, 0);
  assert.deepEqual(choice, { picks: [4, 12], hidden: [30] }, 'removing an automatic region hides it');
  assert.deepEqual(readRegionChoice(writeRegionChoice(choice)), choice);
  assert.deepEqual(removeRegion(choice, 12), { picks: [4], hidden: [30, 12] }, 'removing a pick also hides it, so an active one does not come straight back under a button that said Remove');
  assert.deepEqual(addRegion(removeRegion(choice, 12), 12), { picks: [4, 12], hidden: [30] }, 'Add is its inverse');
  assert.deepEqual(addRegion(choice, 30), { picks: [4, 12, 30], hidden: [] }, 'choosing a hidden region picks it');
  assert.deepEqual(regionParams(choice), { pinnedRegions: [4, 12], hiddenRegions: [30] });
  const stored = setOption(fields, undefined, 'regions', writeRegionChoice(choice));
  assert.equal(resolveOptions(fields, stored!).regions, writeRegionChoice(choice));

  const row = (id: number, picked: boolean, top: number) => ({ key: `r${id}`, id, label: `R${id}`, picked, top, height: 26, active: !picked });
  const shown: RegionRowsField = { rows: [row(4, true, 20), row(12, true, 50), row(7, false, 100), row(9, false, 130)], add: { top: 160, height: 26 }, nameLeft: 8, nameRight: 176 };
  assert.deepEqual(regionDropAt(shown, shown.rows[1]!, 22), { to: 0, lineY: 20 }, 'a pick dragged above the first pick leads');
  assert.deepEqual(regionDropAt(shown, shown.rows[2]!, 55), { to: 1, lineY: 50 }, 'an automatic row dragged among the picks becomes a pick');
  assert.equal(regionDropAt(shown, shown.rows[3]!, 140), null, 'an automatic row dropped among the automatic rows does nothing');
  assert.deepEqual(keyboardMove(shown.rows[0]!, 1, choice), { picks: [12, 4], hidden: [30] });
  assert.equal(keyboardMove(shown.rows[0]!, -1, choice), null);
  assert.deepEqual(keyboardMove(shown.rows[2]!, -1, choice), { picks: [4, 12, 7], hidden: [30] }, 'Alt+Up on an automatic row picks it');

  const list = { kind: 'ready' as const, regions: [
    { id: 7, label: 'Baltic Sea', nameMissing: false, sea: true },
    { id: 30, label: 'North Sea', nameMissing: false, sea: true },
    { id: 31, label: 'Skagerrak', nameMissing: false, sea: true },
    { id: 4, label: 'Western Germany', nameMissing: false, sea: false },
  ] };
  const inChart = [{ id: 4, label: 'Western Germany', active: false }, { id: 7, label: 'Baltic Sea', active: true }];
  const groups = regionChooserGroups(list, inChart, { ids: [30, 31], missing: null }, false, '');
  assert.deepEqual(groups.map(group => [group.title, group.rows.map(item => [item.label, item.inChart])]), [
    ['Active now', [['Baltic Sea', true], ['North Sea', false], ['Skagerrak', false]]],
    ['Land regions', [['Western Germany', true]]],
    ['Sea regions', [['Baltic Sea', true], ['North Sea', false], ['Skagerrak', false]]],
  ], 'Active now lists what the product says is active at the hour, also past the top or removed; a row in the chart is marked, not given its readout');
  const unknown = regionChooserGroups(list, inChart, { ids: [], missing: 'Air scores were read only in part at this hour' }, true, '');
  assert.deepEqual([unknown[0]!.title, unknown[0]!.note], ['Active now', 'Air scores were read only in part at this hour'], 'activity not known at the hour is said, not shown as none');
  assert.equal(regionChooserGroups(list, inChart, null, true, '')[0]!.title, 'Active in this chart', 'without the product answer the group names only what the chart holds');
  assert.deepEqual([savedRegionName({ kind: 'loading' }, 99), savedRegionName({ kind: 'failed', failure: {} as never, retry: () => {} }, 99), savedRegionName(list, 99), savedRegionName(list, 7)],
    ['Region 99 · loading its name…', 'Region 99 · its name could not be loaded', 'Region 99 · not in this recording', 'Baltic Sea'], 'a saved pick is "not in this recording" only once the list says so');
  const opening = { owner: 'air:GER', hour: 400, rows: inChart, active: null };
  assert.equal(chooserFor(opening, 'air:GER'), opening);
  assert.equal(chooserFor(opening, 'air:ITA'), null, 'after the owner changes the old list is never shown or applied (R4-S10)');

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
  const rowHeightOption: OptionField = { key: 'rowHeight', section: 'options', label: 'Row height', kind: 'choice', default: 'normal', choices: ROW_HEIGHT_CHOICES };
  const barsBehindOption: OptionField = { key: 'barsBehind', section: 'options', label: 'Bars behind figures', kind: 'choice', default: 'all', choices: BARS_BEHIND_CHOICES };
  const entries: readonly FamilyViewEntry[] = [
    {
      kind: 'synthetic-dual',
      view: 'table',
      label: 'Table',
      family: 'table',
      options: [rowHeightOption, barsBehindOption],
      loadSpec: async () => ({ family: 'table', kind: 'synthetic-dual', view: 'table', projection: { family: 'table', kind: 'synthetic-dual', project: () => ({}) } }),
    },
    {
      kind: 'synthetic-dual',
      view: 'bars',
      label: 'Bars',
      family: 'bars',
      options: [regionPickerControl(), rowHeightOption],
      loadSpec: async () => ({ family: 'bars', kind: 'synthetic-dual', view: 'bars', projection: { family: 'bars', kind: 'synthetic-dual', project: () => ({}) } }),
    },
  ];
  const choices = declaredViewChoices(undefined, entries);
  assert.deepEqual(choices.map(item => [item.value, item.label, item.family]), [
    ['table', 'Table', 'table'],
    ['bars', 'Bars', 'bars'],
  ]);
  assert.equal(resolveActiveView(undefined, undefined, entries), 'table');
  assert.equal(resolveActiveView({ view: 'bars' }, undefined, entries), 'bars');
  assert.deepEqual(scopedOptionFields(undefined, 'table', entries).map(field => field.key), ['view', 'rowHeight', 'barsBehind']);
  assert.deepEqual(scopedOptionFields(undefined, 'bars', entries).map(field => field.key), ['view', 'regions', 'rowHeight']);
  const tableFields = scopedOptionFields(undefined, 'table', entries);
  const barsFields = scopedOptionFields(undefined, 'bars', entries);
  const rowHeightField = tableFields.find(field => field.key === 'rowHeight')!;
  const otherValue = rowHeightField.kind === 'choice' ? rowHeightField.choices.find(choice => choice.value !== rowHeightField.default)!.value : null;
  assert.ok(otherValue);
  const stored = setOption(tableFields, { view: 'table' }, 'rowHeight', otherValue)!;
  assert.equal(resolveOptions(tableFields, stored).rowHeight, otherValue, 'the table view reads the row height it stored');
  assert.equal(resolveOptions(barsFields, { ...stored, view: 'bars' }).rowHeight, rowHeightField.default, 'the bars view keeps its own row height');
  assert.equal(resolveOptions(barsFields, { ...stored, view: 'bars' }).view, 'bars', 'the view switch itself is shared by the kind');
  const picked = setOption(barsFields, { view: 'bars' }, 'regions', 'r:3a:16')!;
  assert.equal(resolveOptions(viewOwnedFields([regionPickerControl()], 'timeline'), picked).regions, 'r:3a:16', 'every view shows the same regions in the same order');
  assert.deepEqual(manifestViewSwitch(undefined, entries), {
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
  assert.equal(ensureFamilyProjection('table', 'synthetic-same-family', undefined, () => {}), 'unknown', 'a kind with named views has no default view to fall back to');
});

test('a preset is materialised once through the mapping of the view it opens, and the catalogue preview and the placed widget are that one record (R3-S3, R4-S4)', () => {
  const initial = { title: 'World tension contributions', options: { values: 'recorded' } };
  const shown = presetWidget('world-tension', initial);
  assert.equal(presetWidget('world-tension', initial), shown, 'R4-S4: preview and Add get the same materialised preset');
  const stored = shown.options!;
  assert.equal(shown.title, 'World tension contributions');
  assert.equal(resolveWidgetActiveView('world-tension', stored), 'tension');
  assert.equal(widgetViewOptions('world-tension', stored).values, 'recorded', 'the Tension view shows recorded contributions, not its default share');
  const race = presetWidget('army-size', { options: { view: 'race', countFor: 'host' } }).options!;
  assert.equal(widgetViewOptions('army-size', race).countFor, 'host', 'R3-S2: a race preset counting expeditionary forces for the host is what the race view and its detail read');
  assert.equal(viewSlotProps('army-size', { widget: { kind: 'army-size', options: race } as unknown as WidgetSpec }).options.countFor, 'host');
});

test('every setting a time-chart view shows reaches what that view reads or projects (rv11 S19)', async () => {
  const silent: string[] = [];
  const checked: string[] = [];
  const owned = new Set(['political-power', 'command-power', 'stability', 'fuel', 'experience', 'supply-health', 'ruling-party',
    'army-size:lines', 'navy-size:lines', 'air-force-size:lines', 'factories:lines', 'occupation-trend:lines',
    'equipment-stockpile:time', 'resource-balance:time']);
  for (const entry of FAMILY_VIEWS.filter(item => item.family === 'time-chart' && (owned.has(item.kind) || owned.has(`${item.kind}:${normalizeView(item.view)}`)))) {
    const view = normalizeView(entry.view);
    const spec = await entry.loadSpec();
    const declaration = spec.declaration as {
      readonly sources: (ctx: unknown, viewport: unknown) => unknown;
      readonly projectionOptions: (ctx: unknown, extra: unknown) => unknown;
    };
    const selected: WidgetOptions = view === DEFAULT_VIEW ? {} : { [entry.viewOption ?? 'view']: view };
    const read = familyViewOptionFields(entry.kind, view);
    const fields = read;
    const seen = (stored: WidgetOptions, following = false) => {
      const options = resolveOptions(read, stored);
      const ctx = {
        track: { source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 1000, gaps: [] }, axisOffset: 0 },
        branch: { id: '1', firstHour: 0, lastHour: 1000, gaps: [] }, lastRecordedHour: 1000, widget: { kind: entry.kind, options: stored, countries: { kind: 'tags', tags: ['GER'] } },
        options, following, aux: undefined, countryTag: 'GER', countrySubject: 'GER', countryName: 'Germany',
        countries: { all: false, tags: ['GER'], holders: null, every: ['GER'], top: null, membership: null, membershipRuns: null, unobserved: [],
          filter: { passing: null, unrecorded: [], bounds: [] }, labels: {}, factions: null, factionUnobserved: [], factionIncomplete: [],
          rankedAt: () => ['GER'], reason: null, known: () => true, unobservedIn: () => false },
        gateSpan: () => null, staticSpan: null,
      };
      return JSON.stringify([declaration.sources(ctx, { width: 800, height: 300 }), declaration.projectionOptions(ctx, { extent: [0, 1000], surface: '#111111' })],
        (_key, value) => typeof value === 'function' ? String(value) : typeof value === 'bigint' ? String(value) : value);
    };
    for (const field of fields) {
      const following = field.section === 'time';
      const base = seen(selected, following);
      if (field.key === (entry.viewOption ?? 'view')) continue;
      const value = otherValue(field);
      if (value === field.default) continue;
      const written = setOption(fields, selected, field.key, value);
      if (!written) continue;
      const shown = field.visibleWhen ? field.visibleWhen(resolveOptions(read, written), undefined as never) : true;
      if (!shown) continue;
      checked.push(`${entry.kind}/${view}: ${field.key}`);
      if (seen(written, following) === base) silent.push(`${entry.kind}/${view}: ${field.key}`);
    }
  }
  assert.ok(checked.length > 30, `the walk reaches the views' settings: ${checked.length}`);
  assert.deepEqual(silent, [], 'a setting that changes neither the read nor the projection does nothing: wire it or remove it');
});
