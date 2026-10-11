import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { SupplyHealthReply, CountryGroupReply } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { PlotFrame } from '../../charts/frame';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type StackedShareChart from '../../charts/StackedShareChart';
import type { reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';

test('actual Supply main requests and flags require a dated actor while missing counts and known zero stay distinct', async () => {
  const tag = 'opaque actor';
  const run = { firstHour: 10, lastHour: 10, startsSegment: true, noSupply: 1, strained: 0, inSupply: 0 };
  const reply: SupplyHealthReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [],
    series: [{ tag, identities: [{ firstHour: 10, lastHour: 11, slot: 1, countryId: 2 }, { firstHour: 14, lastHour: 16, slot: 1, countryId: 2 }],
      runs: [run, { ...run, firstHour: 14, lastHour: 14, noSupply: 0 }, { ...run, firstHour: 16, lastHour: 16 }] }] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [],
    countryIdentities: [{ tag, identities: reply.series[0].identities! },
      { tag: 'unselected actor', identities: [{ firstHour: 14, lastHour: 14, slot: 2, countryId: 3 }] }],
    capitulations: [10, 11, 12, 14, 17, 21].map(hour => ({ tag, hour })).concat([{ tag: 'unselected actor', hour: 14 }]),
    unobserved: { humans: [], factions: [], wars: [] } };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__supplyMainReply, retry() {} }); export const loadProduct = () => { throw new Error("Unexpected fact read"); };',
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => { globalThis.__supplyMarkerDemand = active; return { data: ${JSON.stringify(groups)} }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 20; export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = (_client, _track, wanted) => { globalThis.__supplyMainWanted = wanted; return { labelAt: hour => tag => { globalThis.__supplyMainLabelReads.push([hour, tag]); return { name: "Actor at " + hour, ...globalThis.__supplyMainFlag }; }, revision: 0 }; };',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({ error: "bad", warning: "warn", positive: "good" });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/WindowedRows.tsx': 'import { createElement } from "react"; export default props => createElement("div", null, props.items.map((item, slot) => props.children({ ...item, slot, phase: "staying" })));',
    '/charts/StackedShareChart.tsx': 'export default props => { globalThis.__supplyMainChart = props; return null; };',
  };
  const host = globalThis as typeof globalThis & { __supplyMainReply?: SupplyHealthReply; __supplyMainChart?: TimeChartProps;
    __supplyMainFlag?: Pick<ReturnType<CountryLabel>, 'flag' | 'flagReason' | 'flagPending'>; __supplyMainWanted?: { tag: string; hour: number }[];
    __supplyMainHover?: { tag: string; hour: number }; __supplyMainLabelReads?: [number, string][]; __supplyMarkerDemand?: boolean };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'supply-main-actor-fixture', enforce: 'pre',
      transform: (source, id) => {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        // Seed only DOM viewport/hover state; retain the actual requests and tooltip adapter.
        if (id.endsWith('/widgets/supply-health/SupplyHealthWidget.tsx')) return source.replace('useState(0)', 'useState(600)')
          .replace('useState<string[]>([])', 'useState<string[]>(["opaque actor"])')
          .replace('const [tip, setTip] = useState<{ tag: string; hour: number } | null>(null)',
            'const [tip, setTip] = useState<{ tag: string; hour: number } | null>(globalThis.__supplyMainHover)');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/supply-health/SupplyHealthWidget.tsx') as typeof import('./SupplyHealthWidget.tsx');
    const props = { widget: { id: 'supply', kind: 'supply-health', trackIds: ['main'], countries: { kind: 'tags', tags: [tag] }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
        branch: { id: '1', firstHour: 10, lastHour: 20 } }], clock: { getHour: () => 20 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    for (const example of [{ hour: 10, actor: true, flag: { flag: 'synthetic flag' }, value: '100.00% · 1 division' },
      { hour: 11, actor: true, flag: { flagPending: true }, value: 'Not known' },
      { hour: 12, actor: false }, { hour: 17, actor: false },
      { hour: 14, actor: true, flag: { flagReason: 'Supplying flag read refused', flagPending: false }, value: '0' },
      { hour: 11, actor: false, older: true }, { hour: 14, actor: true, older: true, flag: { flagPending: true }, value: '0' }]) {
      host.__supplyMainReply = example.older ? { ...reply, series: reply.series.map(series => ({ tag: series.tag, runs: series.runs })) } : reply;
      host.__supplyMainHover = { tag, hour: example.hour }; host.__supplyMainFlag = example.flag; host.__supplyMainLabelReads = [];
      renderToStaticMarkup(createElement(Widget, props));
      assert.equal(host.__supplyMainWanted!.some(wanted => wanted.tag === tag && wanted.hour === example.hour), example.actor);
      assert.equal(host.__supplyMarkerDemand, true, 'manual Supply charts request recorded capitulations');
      const chart = host.__supplyMainChart!;
      assert.deepEqual(chart.badges?.map(badge => badge.hour), example.older ? [10, 14] : [10, 11, 14]);
      assert.equal(chart.keyHours?.includes(11), !example.older, 'a known-identity event remains keyboard reachable without a numeric read');
      const markers = chart.badges!.filter(badge => badge.hour === 14);
      const eventTip = chart.tooltip!(14, {} as PlotFrame, markers)!;
      assert.equal(eventTip.rows.at(-1)?.label, '\u00d7 Actor at 14 capitulated');
      assert.deepEqual(eventTip.rows.at(-1)?.countryFlag, { name: 'Actor at 14', ...example.flag });
      assert.equal(eventTip.rows[0].value, '0', 'capitulation leaves the known zero untouched');
      const tip = host.__supplyMainChart!.tooltip!(example.hour, {} as PlotFrame, [])!;
      assert.equal(tip.rows.find(row => row.label === 'No supply')?.value ?? tip.rows[0].value, example.actor ? example.value : 'Not known');
      if (example.actor) assert.deepEqual(tip.countryFlag, { name: 'Actor at ' + example.hour, ...example.flag });
      else {
        assert.equal(tip.countryFlag, undefined); assert.match(tip.title, /Unknown country/);
        assert.ok(host.__supplyMainLabelReads!.every(([hour]) => hour !== example.hour));
      }
    }
  } finally {
    delete host.__supplyMainReply; delete host.__supplyMainChart; delete host.__supplyMainFlag;
    delete host.__supplyMainWanted; delete host.__supplyMainHover; delete host.__supplyMainLabelReads; delete host.__supplyMarkerDemand;
    await vite.close();
  }
});

test('actual Supply picks and standalone selection require recorded identity while omitted legacy identity uses complete counts', async () => {
  const tag = 'opaque actor';
  const run = { firstHour: 10, lastHour: 10, startsSegment: true, noSupply: 1, strained: 0, inSupply: 0 };
  type Action = Parameters<typeof reducePartsRaceDrill>[1];
  const actions: Action[] = [], effects: (() => void)[] = [];
  const portals: { label: unknown; hour: number | null; reason: string | null }[] = [];
  const host = globalThis as typeof globalThis & {
    __supplyIdentityReply?: SupplyHealthReply;
    __supplyIdentityChart?: ComponentProps<typeof StackedShareChart>;
    __supplyIdentityWanted?: { tag: string; hour: number }[];
    __supplyIdentityEffect?: (effect: () => void) => void;
    __supplyIdentityDispatch?: (action: Action) => void;
    __supplyIdentityDrill?: Parameters<typeof reducePartsRaceDrill>[0];
    __supplyCountryPortal?: (chip: { props: (typeof portals)[number] }) => null;
  };
  host.__supplyIdentityEffect = effect => { effects.push(effect); };
  host.__supplyIdentityDispatch = action => { actions.push(action); };
  host.__supplyCountryPortal = chip => { portals.push(chip.props); return null; };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__supplyIdentityReply, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 10; export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = (_client, _track, wanted) => { globalThis.__supplyIdentityWanted = wanted; return { labelAt: hour => tag => ({ name: tag + " at " + hour }), revision: 0 }; };',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({ error: "bad", warning: "warn", positive: "good" });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/widgets/supply-health/SupplyBreakdown.tsx': 'export default () => null;',
    '/charts/WindowedRows.tsx': 'import { createElement } from "react"; export default props => createElement("div", null, props.items.map((item, slot) => props.children({ ...item, slot, phase: "staying" })));',
    '/charts/StackedShareChart.tsx': 'export default props => { globalThis.__supplyIdentityChart = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'supply-identity-selection-fixture', enforce: 'pre',
      transform(source, id) {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        // Observe real dispatches and portals; chooser state comes from the real reducer.
        if (id.endsWith('/widgets/supply-health/SupplyHealthWidget.tsx')) return source.replace('useState(0)', 'useState(600)')
          .replace('useState<string[]>([])', 'useState<string[]>(["opaque actor"])')
          .replace('useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE)', '[globalThis.__supplyIdentityDrill ?? CLOSED_PARTS_RACE, globalThis.__supplyIdentityDispatch]')
          .replaceAll('useEffect(', 'globalThis.__supplyIdentityEffect(')
          .replace("import { createPortal } from 'react-dom';", 'const createPortal = globalThis.__supplyCountryPortal;');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/supply-health/SupplyHealthWidget.tsx') as typeof import('./SupplyHealthWidget.tsx');
    const props = { widget: { id: 'supply', kind: 'supply-health', trackIds: ['main'], countries: { kind: 'tags', tags: [tag] }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
        branch: { id: '1', firstHour: 10, lastHour: 10 } }], clock: { getHour: () => 10, getSnapshot: () => ({ playing: false }) }, client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const capturedChart = () => { assert.ok(host.__supplyIdentityChart); return host.__supplyIdentityChart; };
    for (const actor of [false, true]) {
      host.__supplyIdentityReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [],
        series: [{ tag, runs: [run], ...(actor ? {} : { identities: [] }) }] };
      props.widget.options = {};
      actions.length = 0; effects.length = 0;
      renderToStaticMarkup(createElement(Widget, props));
      const chart = capturedChart();
      assert.deepEqual(chart.runs, [{ firstHour: 10, lastHour: 10, startsSegment: true, counts: [1, 0, 0] }], 'identity refusal preserves exact numeric geometry');
      assert.deepEqual(host.__supplyIdentityWanted, actor ? [{ tag, hour: 10 }] : []);
      chart.onPick!(10);
      const selected: Action[] = actor ? [{ type: 'open', breakdown: tag, hour: 10, playing: false, playhead: null }] : [];
      assert.deepEqual(actions, selected, 'the registered chart pick never substitutes a numeric run for a present empty identity list');

      props.widget.options = { show: 'breakdown' };
      actions.length = 0; effects.length = 0;
      renderToStaticMarkup(createElement(Widget, props));
      for (const effect of effects.splice(0)) effect();
      assert.deepEqual(actions.filter(action => action.type === 'open' || action.type === 'actors'), selected,
        'standalone actor selection uses complete numeric coverage only when the historical identity field was omitted');
    }
    const identity = { firstHour: 10, lastHour: 10, slot: 1, countryId: 2 };
    const restored: { label: string; origin: number; identities?: SupplyHealthReply['series'][number]['identities']; actor: boolean }[] = [
      { label: 'present empty identities', origin: 10, identities: [], actor: false },
      { label: 'holder present later but missing at origin', origin: 10, identities: [{ firstHour: 11, lastHour: 11, slot: 7, countryId: 9 }], actor: false },
      { label: 'valid origin followed by a renamed alias', origin: 10, identities: [identity], actor: true },
      { label: 'omitted legacy identity with complete origin count', origin: 10, actor: true },
      { label: 'omitted legacy identity without origin count', origin: 9, actor: false },
    ];
    props.runtime.tracks[0].branch.firstHour = 9;
    props.runtime.tracks[0].branch.lastHour = 11;
    for (const example of restored) {
      host.__supplyIdentityReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [], series: [
        { tag, runs: [run], ...(example.identities === undefined ? {} : { identities: example.identities }) },
        { tag: 'renamed later alias', identities: [{ ...identity, firstHour: 11, lastHour: 11 }], runs: [{ ...run, firstHour: 11, lastHour: 11 }] },
      ] };
      props.widget.options = { show: 'breakdown', drillOriginHour: example.origin };
      actions.length = 0; effects.length = 0;
      renderToStaticMarkup(createElement(Widget, props));
      for (const effect of effects.splice(0)) effect();
      assert.deepEqual(actions.filter(action => action.type === 'open' || action.type === 'actors'), example.actor
        ? [{ type: 'open', breakdown: tag, hour: 11, playing: false, playhead: null }] : [],
        `saved native selection checks the original tag at its origin, independently of its later alias: ${example.label}`);
    }
    host.__supplyIdentityReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [], series: [
      { tag, identities: [{ ...identity, firstHour: 9 }, { ...identity, firstHour: 11, lastHour: 11, slot: 3, countryId: 4 }],
        runs: [run, { ...run, firstHour: 11, lastHour: 11 }] },
      { tag: 'renamed later alias', identities: [{ ...identity, firstHour: 11, lastHour: 11 }], runs: [{ ...run, firstHour: 11, lastHour: 11 }] },
    ] };
    props.countrySlot = {} as HTMLElement;
    props.widget.options = { show: 'breakdown' };
    props.widget.time = { mode: 'campaign' };
    actions.length = 0; effects.length = 0;
    renderToStaticMarkup(createElement(Widget, props));
    for (const effect of effects.splice(0)) effect();
    const chooser = actions.find(action => action.type === 'actors');
    assert.ok(chooser, 'a renamed holder and the replacement using its old tag require an explicit actor choice');
    const reducer = await vite.ssrLoadModule('/ui/src/charts/race/partsRaceDrill.ts') as typeof import('../../charts/race/partsRaceDrill.ts');
    host.__supplyIdentityDrill = reducer.reducePartsRaceDrill(reducer.CLOSED_PARTS_RACE, chooser);
    portals.length = 0;
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /Choose a country for/);
    assert.deepEqual(portals, [{ label: null, hour: 11, reason: 'Selected country unavailable.' }]);
    host.__supplyIdentityDrill = undefined;
    props.widget.time = { mode: 'pin', windows: { main: { firstOffset: 1, lastOffset: 1 } } };
    for (const origin of ['10', 10.5, null, 8, 12]) {
      props.widget.options = { show: 'breakdown', drillOriginHour: origin };
      actions.length = 0; effects.length = 0; portals.length = 0;
      const html = renderToStaticMarkup(createElement(Widget, props));
      for (const effect of effects.splice(0)) effect();
      assert.match(html, /Selected country unavailable/);
      assert.deepEqual(actions.filter(action => action.type === 'open' || action.type === 'actors'), [],
        'a malformed saved origin never selects a current or renamed actor');
      assert.deepEqual(portals, [{ label: null, hour: 10, reason: 'Selected country unavailable.' }]);
    }
    props.widget.options = { show: 'breakdown', drillOriginHour: 10 };
    props.widget.time = { mode: 'pin', windows: {} };
    portals.length = 0;
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /Choose another time in widget settings/);
    assert.equal(portals.length, 1); assert.equal(portals[0].label, null); assert.equal(portals[0].hour, null);
  } finally {
    delete host.__supplyIdentityReply; delete host.__supplyIdentityChart; delete host.__supplyIdentityWanted;
    delete host.__supplyIdentityEffect; delete host.__supplyIdentityDispatch;
    delete host.__supplyIdentityDrill;
    delete host.__supplyCountryPortal;
    await vite.close();
  }
});
