import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';

test('actual Army breakdown renders a country template roster when its deployed count is zero', async () => {
  const span = { firstHour: 0, lastHour: 100 };
  const reply = { branchId: '1', available: true, measures: ['divisions'], unknownMeasures: [], types: [], warnings: [],
    windows: [], window: 0, interval: span, series: [{ tag: 'AAA', membership: null, segments: [span],
      identities: [{ ...span, slot: 1, countryId: 11 }], runs: [[0, 0, 100, '0']] }] };
  const manifest = { ...reply, windows: [span], window: null, interval: null, series: [] };
  const roster = { ...manifest, windows: [], templateSections: ['Unused template with holes', 'Staff and garrison priority: -3', 'Queued HQ without deployed units'] };
  const host = globalThis as typeof globalThis & { __armyTemplateRequest?: { params: { templateAt: [string, number] } } };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => { globalThis.__armyTemplateRequest = request; return { data: ${JSON.stringify(roster)}, retry() {} }; };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: new Map([[0, ${JSON.stringify(reply)}]]), errors: new Map(), retry() {} });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => () => null });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['blue'] });",
    '/charts/race/CountryRaceView.tsx': 'export default props => props.beforeChart;',
    '/charts/drill/DrillParts.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'army-template-roster-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Breakdown } = await vite.ssrLoadModule('/ui/src/widgets/army-size/ArmyBreakdown.tsx') as typeof import('./ArmyBreakdown.tsx');
    const props = { runtime: { client: {} }, track: { source: { id: 'synthetic', revision: '1' }, branch: { id: '1' } },
      active: true, options: {}, countrySlot: null, headerSelection: null, extent: [0, 100], playhead: 50,
      tag: 'AAA', hour: 50, measure: 'divisions', countFor: 'owner', manifest, following: false, pinned: false,
      originHour: 0, onPick() {}, onFollow() {}, onWarnings() {} } as unknown as Parameters<typeof Breakdown>[0];
    const html = renderToStaticMarkup(createElement(Breakdown, props));
    assert.deepEqual(host.__armyTemplateRequest?.params.templateAt, ['AAA', 50]);
    for (const text of ['Division templates', ...roster.templateSections]) assert.ok(html.includes(text), html);
  } finally { delete host.__armyTemplateRequest; await vite.close(); }
});

test('actual Army keeps dated flags, full inventories and focusable chart return targets', async () => {
  type Action = Parameters<typeof reducePartsRaceDrill>[1];
  const actions: Action[] = [], effects: (() => void)[] = [];
  const portals: { label: unknown; hour: number | null; reason: string | null }[] = [];
  const window = { firstHour: 0, lastHour: 100 };
  const types = Array.from({ length: 5000 }, (_, index) => ({ key: `mod group ${index}`, name: `Mod group ${index}` }));
  const reply = { branchId: '1', available: true, measures: ['divisions', 'battalions'], unknownMeasures: [], types, warnings: [],
    windows: [], window: 0, interval: window, series: [{ tag: 'opaque actor', membership: null, segments: [window], runs: [[0, 3, 100]],
      mix: [{ ...window, values: types.map((_, index) => index === 0 ? 3 : 0) }] }] };
  const manifest = { ...reply, windows: [window], window: null, interval: null, series: [] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [],
    countryIdentities: [{ tag: 'opaque actor', identities: [{ ...window, slot: 1, countryId: 10 }] },
      { tag: 'unselected actor', identities: [{ ...window, slot: 2, countryId: 20 }] }],
    capitulations: [{ tag: 'opaque actor', hour: 40 }, { tag: 'unselected actor', hour: 40 }, { tag: 'opaque actor', hour: 101 }],
    unobserved: { humans: [], factions: [], wars: [] } };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(manifest)}, retry() {} });
      export const loadProduct = () => { throw new Error('Unexpected country request'); };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: new Map([[0, ${JSON.stringify(reply)}]]), errors: new Map(), retry() {} });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => { globalThis.__armyMarkerDemand = active; return { data: ${JSON.stringify(groups)} }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 100; export const usePlaybackSpeed = () => 0;',
    '/charts/drill/DrillHost.tsx': 'export default props => { globalThis.__armyFocusSelector = props.returnFocus; return props.main; };',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/widgets/army-size/ArmyBreakdown.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__armyFlagView = props; return null; };',
  };
  const host = globalThis as typeof globalThis & { __armyFlagView?: CountryRaceViewProps; __armyFocusSelector?: string; __armyMarkerDemand?: boolean;
    __armyDrillEffect?: (effect: () => void) => void; __armyDrillDispatch?: (action: Action) => void;
    __armyCountryPortal?: (chip: { props: (typeof portals)[number] }) => null };
  host.__armyDrillEffect = effect => { effects.push(effect); };
  host.__armyDrillDispatch = action => { actions.push(action); };
  host.__armyCountryPortal = chip => { portals.push(chip.props); return null; };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'army-line-flag-fixture', enforce: 'pre',
      transform: (source, id) => {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        if (id.endsWith('/widgets/army-size/ArmySizeWidget.tsx')) return source
          .replace('useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE)', '[CLOSED_PARTS_RACE, globalThis.__armyDrillDispatch]')
          .replaceAll('useEffect(', 'globalThis.__armyDrillEffect(')
          .replace("import { createPortal } from 'react-dom';", 'const createPortal = globalThis.__armyCountryPortal;');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/army-size/ArmySizeWidget.tsx') as typeof import('./ArmySizeWidget.tsx');
    const props = { widget: { id: 'army', kind: 'army-size', trackIds: ['main'], countries: { kind: 'tags', tags: ['opaque actor'] },
      time: { mode: 'campaign' }, options: { view: 'lines' } }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: { id: '1', ...window } }],
      clock: { getHour: () => 100 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(Widget, props));
    const view = host.__armyFlagView!;
    assert.equal(host.__armyMarkerDemand, true, 'manual time views request the existing capitulation history');
    assert.deepEqual(view.badges, [{ hour: 40, glyph: '\u00d7', label: 'Country capitulated', capitulationTag: 'opaque actor' }]);
    assert.deepEqual(view.lines[0].runs.map(run => [run.firstHour, run.lastHour, run.value]), [[0, 100, 3]], 'the selected series continues after capitulation');
    const reads: [number, string][] = [];
    for (const flag of [{ flagPending: true }, { flagPending: false, flagReason: 'Supplying flag read refused' }, { flag: 'synthetic flag' }]) {
      const label = { name: 'Actor at 40', ...flag };
      const labelAt = (hour: number): CountryLabel => tag => { reads.push([hour, tag]); return label; };
      const [tip] = view.lineTooltip!(40, ['opaque actor', 'unobserved actor'], labelAt, () => 'blue', new Map());
      assert.equal(tip.rows.length, 1);
      assert.equal(tip.rows[0].label, 'Actor at 40');
      assert.deepEqual(tip.rows[0].countryFlag, label);
      assert.equal(tip.rows[0].icon, undefined);
    }
    assert.deepEqual(reads, Array.from({ length: 3 }, () => [40, 'opaque actor']));
    const [gap] = view.lineTooltip!(101, ['opaque actor'], () => () => { throw new Error('Unknown actor must not request a label'); }, () => 'blue', new Map());
    assert.equal(gap.rows.length, 0);
    const inventoryOptions: WidgetProps['widget']['options'][] = [{ view: 'race', measure: 'battalions', byType: true }, { view: 'mix' }];
    for (const options of inventoryOptions) {
      props.widget.options = options;
      renderToStaticMarkup(createElement(Widget, props));
      const main = host.__armyFlagView!;
      assert.equal(main.parts?.length, 5000, 'every authored group reaches the shared view');
      assert.equal(main.presentation?.windowedLegend, true);
      assert.equal(host.__armyMarkerDemand, main.view !== 'race');
      if (main.view === 'mix') {
        assert.deepEqual(main.badges, view.badges);
        assert.equal(main.mix?.compactStack, true);
        assert.equal(main.mix?.parts?.length, 5000);
        assert.equal(main.mix?.bands?.length, 5000);
      } else { assert.equal(main.raceLegendParts?.length, 5000); assert.equal(main.badges, undefined); }
    }
    props.countrySlot = {} as HTMLElement;
    props.widget.time = { mode: 'pin', windows: { main: { firstOffset: 50, lastOffset: 50 } } };
    for (const origin of ['50', 50.5, null, -1, 101]) {
      props.widget.options = { show: 'breakdown', drillOriginHour: origin };
      actions.length = 0; effects.length = 0; portals.length = 0;
      const html = renderToStaticMarkup(createElement(Widget, props));
      for (const effect of effects.splice(0)) effect();
      assert.match(html, /Saved country unavailable/);
      assert.deepEqual(actions.filter(action => action.type === 'open' || action.type === 'actors'), [],
        'a malformed saved origin never selects the currently recorded actor');
      assert.deepEqual(portals, [{ label: null, hour: 50, reason: 'Saved country unavailable.' }]);
    }
    props.widget.options = { show: 'breakdown', drillOriginHour: 50 };
    props.widget.time = { mode: 'pin', windows: {} };
    portals.length = 0;
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /Choose another time in widget settings/);
    assert.equal(portals.length, 1); assert.equal(portals[0].label, null); assert.equal(portals[0].hour, null);
    const { default: RaceChart } = await vite.ssrLoadModule('/ui/src/charts/race/RaceChart.tsx') as typeof import('../../charts/race/RaceChart');
    const { default: TimeChart } = await vite.ssrLoadModule('/ui/src/charts/TimeChart.tsx') as typeof import('../../charts/TimeChart');
    for (const chart of [
      createElement(RaceChart, { label: 'Army race', items: [], slots: 1, name: key => key, active: false }),
      createElement(TimeChart, { label: 'Army history', extent: [0, 100], playhead: null, playback: 'cursor', branchStart: 0,
        axes: { xFrom: null, xTo: null, yMin: null, yMax: null, log: false }, layers: [], active: false, revision: 0 }),
    ]) {
      const markup = renderToStaticMarkup(chart);
      // Model only the caller's class/descendant selector inside this owned chart.
      // The real rendered attributes distinguish the wrapper from its keyboard target.
      const rootClass = /^<div class="([^"]+)">/.exec(markup)?.[1];
      const selected = host.__armyFocusSelector!.split(',').flatMap(selector => {
        const [parent, child] = selector.trim().split(/\s+/);
        if (parent !== `.${rootClass}`) return [];
        const tag = child ?? 'div';
        return [...markup.matchAll(/<(div|canvas)\b[^>]*>/g)].filter(match => match[1] === tag).slice(0, 1);
      })[0];
      assert.ok(selected, 'the return target exists inside the owned chart');
      assert.equal(selected[1], 'canvas', 'the actual Army caller must select the keyboard surface, not the wrapper');
      assert.match(selected[0], /\btabindex="0"/, 'the selected actual rendered surface accepts keyboard focus');
    }
  } finally {
    delete host.__armyFlagView;
    delete host.__armyFocusSelector;
    delete host.__armyMarkerDemand;
    delete host.__armyDrillEffect; delete host.__armyDrillDispatch; delete host.__armyCountryPortal;
    await vite.close();
  }
});
