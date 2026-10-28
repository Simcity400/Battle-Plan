import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentType, ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../../charts/frame.ts';
import { ChartPicks } from '../../charts/pick.ts';
import type { LegendItem } from '../../charts/Legend';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { CountryStabilityReply, StabilityPartsReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';

const native: CountryStabilityReply = { branchId: 'branch', decimals: 5, warnings: [],
  series: ['actor A', 'actor B'].map((tag, index) => ({ tag, recorded: true, carried: [], breakdown: null, maxCommandPower: [],
    stability: [{ firstHour: 0, lastHour: 100, startsSegment: true, valueRaw: index ? '80000' : '10000' }],
    warSupport: [{ firstHour: 0, lastHour: 100, startsSegment: true, valueRaw: index ? '30000' : '80000' }],
  })) };
const detail = (tag: string): StabilityPartsReply => ({ branchId: 'branch', tag, decimals: 5, warnings: [],
  stability: { observed: [{ firstHour: 0, lastHour: 100 }], parts: [] },
  warSupport: { observed: [{ firstHour: 0, lastHour: 100 }], parts: [] } });

test('the registered Stability renderer opens a drawn comparison chooser and pins its selected actor and held day', async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[] };
  const state = { cursor: 0, slots: [] as Slot[], effects: [] as (() => unknown)[], charts: [] as TimeChartProps[],
    host: undefined as { open: boolean; day: string; canFollow: boolean; onPin?: () => void; onFollow: () => void } | undefined,
    picker: undefined as { allowedTags: string[]; onChange: (tag: string) => void } | undefined,
    legend: undefined as { windowed: boolean; items: LegendItem[]; onVisibleItems: (items: readonly LegendItem[]) => void } | undefined,
    requestedFlags: [] as readonly string[], flagPending: true, flagReason: undefined as string | undefined,
    segmentAt: undefined as ((tag: string, hour: number) => number | null) | undefined,
    data: native as CountryStabilityReply | undefined, clock: 100, name: '',
  };
  const host = globalThis as typeof globalThis & { __stabilityRenderer?: typeof state;
    __stabilityHook?: (kind: string, ...args: unknown[]) => unknown };
  host.__stabilityRenderer = state;
  // Keep the renderer's actual hook slots and callbacks across SSR renders. Only
  // transport, sampled artwork and leaf surfaces are replaced; reducers stay real.
  host.__stabilityHook = (kind, ...args) => {
    const index = state.cursor++;
    const slot = state.slots[index] ??= {};
    if (kind === 'useMemo' || kind === 'useEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      const same = slot.deps?.length === deps.length && slot.deps.every((value, at) => Object.is(value, deps[at]));
      if (!same) { slot.deps = [...deps]; if (kind === 'useMemo') slot.value = factory(); else state.effects.push(factory); }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = kind === 'useReducer' ? args[1] : kind === 'useRef' ? { current: args[0] }
      : typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    if (kind === 'useRef') return slot.value;
    const set = (next: unknown) => { slot.value = kind === 'useReducer'
      ? (args[0] as (previous: unknown, action: unknown) => unknown)(slot.value, next)
      : typeof next === 'function' ? (next as (previous: unknown) => unknown)(slot.value) : next; };
    return [slot.value, set];
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product === 'country-stability'
      ? globalThis.__stabilityRenderer.data && { ...globalThis.__stabilityRenderer.data, series:
        globalThis.__stabilityRenderer.data.series.filter(row => request.params.tags.includes(row.tag)) } : request?.product === 'stability-parts'
      ? { ...${JSON.stringify(detail(''))}, tag: request.params.tag } : undefined, retry() {} });
      export const useProducts = (client, requests) => ({data: requests.length ? requests.map(request => useProduct(client, request).data) : undefined, retry() {}});
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/useCountryGroups.ts': `const data = {branchId: 'branch', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
      countryIdentities: ['actor A', 'actor B'].map((tag, index) => ({tag, identities: [{firstHour: 0, lastHour: 101, slot: index + 1, countryId: index + 10}]})),
      unobserved: {humans: [], factions: [], wars: []}}; export const useCountryGroups = (_runtime, _track, active) => ({data: active ? data : undefined});`,
    '/dashboard/campaignSettings.ts': 'export const useCampaignSettings = () => ({campaigns: {}});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__stabilityRenderer.clock;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, _tags, _hour, _active, segmentAt, _hours, _exact, flagTags) => {
      globalThis.__stabilityRenderer.segmentAt = segmentAt; globalThis.__stabilityRenderer.requestedFlags = flagTags; return { name: tag => globalThis.__stabilityRenderer.name + tag,
      color: () => [20, 50, 80], flag: () => undefined, flagPending: () => globalThis.__stabilityRenderer.flagPending,
      flagReason: () => globalThis.__stabilityRenderer.flagReason, revision: 0 }; };`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/widgets/national-focus/useFocusArtwork.ts': 'export const useFocusText = () => ({read: new Map(), warnings: []});',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map(); export const useCountryNamesRead = () => new Map();',
    '/charts/useChartTokens.ts': `const tokens = { categories: ['#123456'], muted: '#888888', surface: '#000000',
      countryColours: 'game', fontSize: { body: 12 } }; export const useChartTokens = () => tokens; export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__stabilityRenderer.picker = props; return null; };',
    '/charts/Legend.tsx': 'export default props => { globalThis.__stabilityRenderer.legend = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__stabilityRenderer.host = props; return createElement('section', null, props.main, props.open ? props.children : null); };`,
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__stabilityRenderer.charts.push(props); return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'stability-drawn-actor', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/stability/StabilityWidget.tsx')) return source.replace(/\b(useMemo|useState|useReducer|useRef|useEffect)(?:<[^>]+>)?\(/g,
          (_match, kind) => `globalThis.__stabilityHook(${JSON.stringify(kind)}, `);
      },
    }] });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const { default: registered } = await vite.ssrLoadModule('/ui/src/widgets/stability/index.ts');
    const Widget = registered.render as ComponentType<WidgetProps>;
    const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [], branches: [] } },
      branch: { id: 'branch', firstHour: 0, lastHour: 100, gaps: [] } };
    const created: WidgetSpec[] = [];
    const props = { widget: { id: 'stability', kind: 'stability', title: 'Stability', configVersion: 1, trackIds: ['track'],
      countries: { kind: 'tags', tags: ['actor A', 'actor B'] }, time: { mode: 'campaign' }, options: { endLabels: false } },
      runtime: { client: {}, tracks: [track], clock: { getHour: () => state.clock, getSnapshot: () => ({ playing: true, hoursPerSecond: 24 }) },
        addWidgetBelow: (_id: string, widget: WidgetSpec) => created.push(widget) }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      state.cursor = 0; state.charts = []; state.effects = []; state.picker = undefined; state.host = undefined;
      renderToStaticMarkup(createElement(Widget, props));
      for (const effect of state.effects) effect();
      return state.charts[0];
    };
    const first = render();
    assert.equal(state.legend!.windowed, true);
    assert.equal(state.legend!.items.length, 2);
    assert.deepEqual(state.requestedFlags, [], 'flags wait for mounted legend entries');
    assert.match(renderToStaticMarkup(state.legend!.items[0].flag as ReactElement), /Flag loading/);
    state.legend!.onVisibleItems([state.legend!.items[1]]);
    render();
    assert.deepEqual(state.requestedFlags, ['actor B']);
    state.flagPending = false; state.flagReason = 'Synthetic recorded-stack flag refusal';
    render();
    assert.doesNotMatch(renderToStaticMarkup(state.legend!.items[1].flag as ReactElement), /Synthetic recorded-stack flag refusal/);
    state.legend!.onVisibleItems([]);
    render();
    assert.deepEqual(state.requestedFlags, [], 'legend unmount or scroll release removes its flag reads');
    state.legend!.onVisibleItems(state.legend!.items);
    render();
    assert.deepEqual(state.requestedFlags, ['actor A', 'actor B']);
    assert.equal(state.segmentAt!('actor A', 0), 0);
    assert.equal(state.segmentAt!('unknown', 20), null);
    first.onPick!(20);
    render();
    assert.equal(state.host!.open, false, 'no drawn frame cannot open even a comparison drill');
    const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
    const canvas = new ChartPicks();
    const update = (chart: TimeChartProps) => canvas.update({ owner: chart.pickOwner, extent: chart.extent,
      branchStart: chart.branchStart, active: chart.active, onPick: chart.onPick });
    update(first);
    const drawn = canvas.drawn({ frame, until: null, point: { x: frame.toX(20), y: frame.toY(80) } }, first.onPick);
    state.data = undefined; state.name = 'New label '; state.clock = 99;
    const pending = render();
    assert.equal(pending.pickOwner, first.pickOwner);
    update(pending);
    assert.equal(canvas.pick(drawn, 20), true);
    render();
    assert.equal(state.host!.open, true);
    assert.deepEqual(state.requestedFlags, [], 'an inactive inline main asks for no legend flag artwork');
    assert.equal(state.host!.canFollow, false);
    assert.equal(state.host!.onPin, undefined);
    assert.deepEqual(state.picker!.allowedTags, ['actor A', 'actor B']);
    state.clock = 100;
    state.picker!.onChange('actor A');
    render();
    assert.equal(state.host!.canFollow, true);
    state.host!.onPin!();
    assert.deepEqual(created[0].countries, { kind: 'tags', tags: ['actor A'] });
    assert.deepEqual(JSON.parse(created[0].options!.stabilityCountry as string),
      { campaign: 'campaign', branch: 'branch', tag: 'actor A', originOffset: 20, slot: 1, countryId: 10 });
    assert.equal(created[0].options?.show, 'warSupport', 'the chosen actor supplies its own witnessed measure');
    assert.deepEqual(created[0].time, { mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } });
    state.host!.onFollow();
    render();
    state.host!.onPin!();
    assert.equal(created[1].time.mode, 'follow');
    state.data = native;
    props.widget.countries = { kind: 'tags', tags: ['actor B'] };
    render();
    assert.equal(state.host!.open, false, 'a changed selection cannot paint the previous actor before its close effect');
    render().onPick!(20, { frame, until: null, point: { x: frame.toX(20), y: frame.toY(80) } });
    render();
    assert.equal(state.host!.open, true);
    track.branch.lastHour = 101;
    render();
    assert.equal(state.host!.open, false, 'native extent changes invalidate the held drill before effects');
    props.widget.countries = { kind: 'tags', tags: ['actor A', 'actor B'] };
    state.data = { ...native, series: native.series.map((row, index) => ({ ...row,
      carried: [{ firstHour: 20, lastHour: 100, readHour: index ? 15 : 10 }] })) };
    const carried = render().tooltip!(20, undefined as never, []);
    assert.match(carried!.note!, /As of/);
    assert.ok(carried!.note!.includes(formatDateTime(10)));
    assert.ok(carried!.note!.includes(formatDateTime(15)), 'different countries keep both supplying-date bounds');
    assert.doesNotMatch(carried!.note!, /As read|Click a line|overlapping lines/);
  } finally { await vite.close(); delete host.__stabilityRenderer; delete host.__stabilityHook; }
});
