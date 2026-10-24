import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { ChartPicks } from '../charts/pick.ts';
import { plotFrame } from '../charts/frame.ts';
import { tokensFromValues } from '../charts/tokens.ts';
import type { TimeChartProps } from '../charts/TimeChart';
import type { WidgetProps } from '../dashboard/definitions';
import type { CountryFactoriesReply, CountryGroupReply, EquipmentProducedReply } from '../generated/dashboard';

const group: CountryGroupReply = { branchId: 'branch', humans: [{ tag: 'MOD', firstHour: 0, lastHour: 200 }],
  countryIdentities: [{ tag: 'MOD', identities: [{ firstHour: 0, lastHour: 200, slot: 3, countryId: 9 }] },
    { tag: 'PLAYER', identities: [{ firstHour: 0, lastHour: 200, slot: 1, countryId: 7 }] }],
  wars: [{ first: 'PLAYER', second: 'MOD', firstHour: 0, lastHour: 200 }], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
  unobserved: { humans: [], factions: [], wars: [] }, factions: [{ id: '7', firstHour: 0, lastHour: 200, recordedName: '',
    name: 'First faction name', nameReason: null, names: [{ firstHour: 0, lastHour: 200, name: 'First faction name', reason: null }],
    members: [{ tag: 'MOD', firstHour: 0, lastHour: 200 }], leaders: [{ tag: 'MOD', firstHour: 0, lastHour: 200 }], incomplete: [] }] };
const factories: CountryFactoriesReply = { branchId: 'branch', fields: [{ group: 'civilian', part: 'total' }], warnings: [],
  groups: [{ key: 'civilian', flag: 'mod production', buildings: [{ token: 'mod works', name: 'First works' }] }],
  series: [{ tag: 'MOD', membership: null, identities: [{ firstHour: 0, lastHour: 200, slot: 3, countryId: 9 }],
    segments: [{ firstHour: 0, lastHour: 200 }], reads: [{ firstHour: 0, lastHour: 200, readHour: 0, hourly: true }],
    runs: [{ firstHour: 0, lastHour: 200, values: [12], startsSegment: true }] }] };
const produced: EquipmentProducedReply = { branchId: 'branch', measure: 'total', count: 'units', decimals: 0, grouping: 'type', warnings: [],
  keys: [{ key: 'mod design', typeKey: 'mod design', name: 'First design', names: [], domain: null }], types: [],
  reads: [0, 24], observations: [0], known: [{ firstHour: 0, lastHour: 200 }],
  series: [{ tag: 'MOD', membership: null, identities: [{ firstHour: 0, lastHour: 200, slot: 3, countryId: 9 }],
    firstHours: [0, 24], lastHours: [23, 200], breaks: [0],
    values: [{ key: 0, values: { from: 0, deltas: [12], unknown: [] } }], total: { from: 0, deltas: [12], unknown: [] } }] };

test('actual economy group requests and open drills retain pending/name refreshes and refuse changed membership or source', async () => {
  const state = { data: undefined as unknown, groups: undefined as CountryGroupReply | undefined, charts: [] as TimeChartProps[],
    requests: [] as { params: { tags: string[] } }[], drill: undefined as unknown, origin: null as number | null,
    memos: new Map<string, { deps: readonly unknown[]; value: unknown }[]>(), cursors: new Map<string, number>(), diagnostics: [] as string[] };
  const host = globalThis as typeof globalThis & { __economyGroup?: typeof state;
    __economyGroupMemo?: (id: string, factory: () => unknown, deps: readonly unknown[]) => unknown };
  host.__economyGroup = state;
  // Retain mounted memo/reducer slots in SSR; all resolution, loading branches,
  // native model decoding, shared chart and full-drill markup remain real.
  host.__economyGroupMemo = (id, factory, deps) => {
    const index = state.cursors.get(id) ?? 0; state.cursors.set(id, index + 1);
    let slots = state.memos.get(id); if (!slots) { slots = []; state.memos.set(id, slots); }
    const before = slots[index];
    if (before && before.deps.length === deps.length && before.deps.every((value, i) => Object.is(value, deps[i]))) return before.value;
    const value = factory(); slots[index] = { deps: [...deps], value }; return value;
  };
  const memoModules = ['/widgets/factories/FactoriesWidget.tsx', '/widgets/equipment-produced/EquipmentProducedWidget.tsx',
    '/charts/race/CountryPartsDrill.tsx', '/charts/race/CountryRaceView.tsx'];
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__economyGroup.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {}; export const usePlaybackSpeed = () => 24;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag, color: () => null, flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))});
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__economyGroup.charts.push(props); return createElement('canvas', { 'data-group-chart': true }); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'economy-group-refresh', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return source.slice(0, source.indexOf('export function useProducts<T>'))
          .replace('export async function loadProduct<T>(', 'async function unusedLoadProduct<T>(')
          + `export const useProducts = () => ({ retry() {} });
            export const useProduct = (_client, request, _active, validate) => {
              if (request) globalThis.__economyGroup.requests.push(request);
              return { data: request && globalThis.__economyGroup.data !== undefined ? validate(globalThis.__economyGroup.data) : undefined, retry() {} }; };
            export const loadProduct = (client, request, _signal, validate, _source, _restart, publish) => {
              const { data } = useProduct(client, request, true, validate); if (data !== undefined) publish?.(data); return new Promise(() => {}); };`;
        // The race's view read answers in the same render, as its products do here.
        if (path.endsWith('/dashboard/viewRead.ts')) return `export const useViewRead = (lane, view, _viewKey, active, read) => { let data;
          if (active && lane != null && view != null) void read(view, new AbortController().signal, value => { data = value; });
          return { data, retry() {} }; };`;
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        const module = memoModules.find(suffix => path.endsWith(suffix));
        if (!module) return;
        source = source.replace(/\buseMemo\(/g, `globalThis.__economyGroupMemo(${JSON.stringify(module)}, `);
        if (module.endsWith('CountryPartsDrill.tsx')) source = source.replace('useReducer, useRef, useState', 'useRef')
          .replace('const diagnosticsKey = JSON.stringify(diagnostics);',
            'globalThis.__economyGroup.diagnostics = diagnostics; const diagnosticsKey = JSON.stringify(diagnostics);')
          + `\nconst useReducer = (reduce, initial) => { const state = globalThis.__economyGroup; state.drill ??= initial;
            return [state.drill, action => { state.drill = reduce(state.drill, action); }]; };
          const useState = initial => [globalThis.__economyGroup.origin ?? initial, value => { globalThis.__economyGroup.origin = value; }];`;
        return source;
      },
    }] });
  try {
    const Factory = (await vite.ssrLoadModule('/ui/src/widgets/factories/FactoriesWidget.tsx')).default as ComponentType<WidgetProps>;
    const Produced = (await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx')).default as ComponentType<WidgetProps>;
    for (const [kind, Widget, initial] of [['factories', Factory, factories], ['equipment-produced', Produced, produced]] as const) {
      for (const selection of [{ type: 'humans' }, { type: 'faction', id: '7' }, { type: 'war', tag: 'PLAYER' }] as const) {
        state.memos.clear(); state.drill = undefined; state.origin = null;
        const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: 1, metadata: { countries: [] } },
          branch: { id: 'branch', firstHour: 0, lastHour: 200, gaps: [] } };
        const props = { widget: { id: 'widget', kind, configVersion: 1, trackIds: ['track'],
          countries: { kind: 'group', group: selection, membership: 'hour', aggregate: false }, time: { mode: 'campaign' },
          options: { view: 'lines', count: 'built', endLabels: false } }, runtime: { tracks: [track], client: {},
          clock: { getHour: () => 100, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } },
          active: true, onDiagnostics: (notes: string[]) => { state.diagnostics = notes; } } as unknown as WidgetProps;
        const canvas = new ChartPicks();
        const render = (data?: unknown, groups?: CountryGroupReply) => {
          state.data = data; state.groups = groups; state.charts = []; state.requests = []; state.cursors.clear();
          const html = renderToStaticMarkup(createElement(Widget, props));
          const chart = state.charts.find(chart => chart.active);
          canvas.update(chart ? { owner: chart.pickOwner, extent: chart.extent, branchStart: chart.branchStart, active: chart.active, onPick: chart.onPick }
            : { owner: null, extent: [0, 200], branchStart: 0, active: false });
          return { chart, html };
        };
        const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 200], [0, 20], false);
        const context = { frame, until: null, point: { x: frame.toX(50), y: frame.toY(12) } };
        const first = render(initial, group); assert.ok(first.chart);
        const queued = canvas.drawn(context, first.chart.onPick);
        track.source.revision++;
        const pending = render(); assert.ok(pending.chart, `${kind}/${selection.type}: group refresh keeps chart`);
        assert.equal(pending.chart.pickOwner, first.chart.pickOwner);
        assert.deepEqual(state.requests[0].params.tags, ['MOD']);
        const renamed = { ...group, warnings: ['Group content note'], factions: group.factions.map(faction => ({ ...faction,
          name: 'Later faction name', names: faction.names.map(name => ({ ...name, name: 'Later faction name' })) })) };
        const decorated = structuredClone(initial);
        if ('groups' in decorated) decorated.groups[0].buildings[0].name = 'Later works';
        else decorated.keys[0].name = 'Later design';
        const named = render(decorated, renamed);
        assert.equal(named.chart!.pickOwner, first.chart.pickOwner);
        assert.ok(state.diagnostics.includes('Group content note'));
        const changed = { ...renamed, humans: renamed.humans.map(run => ({ ...run, lastHour: 75 })),
          factions: renamed.factions.map(faction => ({ ...faction, members: faction.members.map(run => ({ ...run, lastHour: 75 })) })),
          wars: renamed.wars.map(run => ({ ...run, lastHour: 75 })) };
        assert.notEqual(render(decorated, changed).chart!.pickOwner, first.chart.pickOwner);
        assert.equal(canvas.pick(queued, 50), false, 'actual membership invalidates the witnessed callback');
        const restored = render(decorated, renamed); assert.ok(restored.chart);
        assert.equal(canvas.pick(canvas.drawn(context, restored.chart.onPick), 50), true);
        const open = render(decorated, renamed); assert.ok(open.chart);
        assert.match(open.html, /data-open="true"/); assert.match(open.html, /Pin as widget/);
        assert.match(open.html, kind === 'factories' ? /Later works/ : /Later design/);
        const dayPick = canvas.drawn(context, open.chart.onPick);
        track.source.revision++;
        const openPending = render(); assert.ok(openPending.chart, 'an open full drill remains mounted during group refresh');
        assert.equal(openPending.chart.pickOwner, open.chart.pickOwner);
        assert.match(openPending.html, />12</);
        assert.equal(render(decorated, renamed).chart!.pickOwner, open.chart.pickOwner);
        assert.equal(canvas.pick(dayPick, 51), true);
        for (const key of ['source', 'branch', 'client', 'selection'] as const) {
          const current = render(decorated, renamed); assert.ok(current.chart);
          const old = canvas.drawn(context, current.chart.onPick);
          if (key === 'source') track.source.id += '-other';
          else if (key === 'branch') track.branch.id += '-other';
          else if (key === 'client') props.runtime.client = { ...props.runtime.client };
          else if (props.widget.countries.kind === 'group') props.widget.countries = { ...props.widget.countries, membership: 'end' };
          assert.equal(render().chart, undefined, `${key}: no prior native data backfills the new scope`);
          assert.equal(canvas.pick(old, 50), false);
        }
      }
    }
  } finally { await vite.close(); delete host.__economyGroup; delete host.__economyGroupMemo; }
});
