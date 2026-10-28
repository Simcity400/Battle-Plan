import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../frame.ts';
import { ChartPicks } from '../pick.ts';
import { tokensFromValues } from '../tokens.ts';
import type { TimeChartProps } from '../TimeChart';
import type { WidgetProps } from '../../dashboard/definitions';
import type { NativePartsRaceReply as PartsRaceReply } from './partsRaceDrill';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { CountryRaceViewProps } from './CountryRaceView';
import type { PartsRaceDrillState } from './partsRaceDrill';

const forces: PartsRaceReply = { branchId: '1', parts: [{ key: 'synthetic part', name: 'First part name', unresolved: false }], warnings: [],
  series: [['OLD', 20, 10], ['NEW', 10, 30]].map(([tag, first, last], index) => ({ tag: String(tag),
    identities: [{ firstHour: 0, lastHour: 100, slot: index + 1, countryId: index + 10 }],
    membership: null, segments: [{ firstHour: 0, lastHour: 100 }], runs: [
      { firstHour: 0, lastHour: 49, startsSegment: true, total: Number(first), values: [Number(first)], referenceTotal: 40 },
      { firstHour: 50, lastHour: 100, startsSegment: false, total: Number(last), values: [Number(last)], referenceTotal: 40 }] })) };
const group: CountryGroupReply = { branchId: '1', humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
  countryIdentities: ['OLD', 'NEW'].map((tag, index) => ({ tag, identities: [{ firstHour: 0, lastHour: 100, slot: index + 1, countryId: index + 10 }] })),
  unobserved: { humans: [], factions: [], wars: [] }, factions: [{ id: '7', firstHour: 0, lastHour: 100, recordedName: '',
    name: 'First faction name', nameReason: null, names: [{ firstHour: 0, lastHour: 100, name: 'First faction name', reason: null }],
    members: ['OLD', 'NEW'].map(tag => ({ tag, firstHour: 0, lastHour: 100 })),
    leaders: [{ tag: 'OLD', firstHour: 0, lastHour: 100 }], incomplete: [] }] };

test('mounted force views keep witnessed actors through pending names and reject changed native contexts', async () => {
  const state = { main: forces as PartsRaceReply | undefined, detail: forces as PartsRaceReply | undefined,
    groups: group as CountryGroupReply | undefined, error: undefined as string | undefined,
    charts: [] as TimeChartProps[], views: [] as Pick<CountryRaceViewProps, 'mix' | 'presentation' | 'moment' | 'playhead'>[],
    lists: [] as { virtual?: boolean; active?: boolean }[], pickers: [] as { allowedTags?: readonly string[]; flagAt?: { hour: number } }[],
    effects: [] as (() => unknown)[], memos: new Map<string, { deps: readonly unknown[]; value: unknown }[]>(),
    cursors: new Map<string, number>(), drill: undefined as PartsRaceDrillState | undefined, diagnostics: [] as string[], name: '', subscriptions: [] as boolean[] };
  const host = globalThis as typeof globalThis & { __forceNumeric?: typeof state;
    __forceMemo?: (id: string, factory: () => unknown, deps: readonly unknown[]) => unknown;
    __forceEffect?: (id: string, effect: () => unknown, deps: readonly unknown[]) => void;
    __forceReducer?: (reducer: (value: PartsRaceDrillState, action: unknown) => PartsRaceDrillState,
      initial: PartsRaceDrillState) => [PartsRaceDrillState, (action: unknown) => void] };
  const drillState = (): PartsRaceDrillState | undefined => state.drill;
  host.__forceNumeric = state;
  // Keep mounted memo/reducer slots across SSR renders; use the real view,
  // loading branches, group resolution, native projections and drawn callbacks.
  host.__forceMemo = (id, factory, deps) => {
    const index = state.cursors.get(id) ?? 0; state.cursors.set(id, index + 1);
    let slots = state.memos.get(id);
    if (!slots) { slots = []; state.memos.set(id, slots); }
    const before = slots[index];
    if (before && before.deps.length === deps.length && before.deps.every((value, i) => Object.is(value, deps[i]))) return before.value;
    const value = factory(); slots[index] = { deps: [...deps], value }; return value;
  };
  host.__forceEffect = (id, effect, deps) => { host.__forceMemo!(id + '/effects', () => state.effects.push(effect), deps); };
  host.__forceReducer = (reducer, initial) => [state.drill ??= initial, action => { state.drill = reducer(state.drill!, action); }];
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__forceNumeric.groups, error: globalThis.__forceNumeric.error, retry() {} });',
    '/dashboard/model.ts': `export const resolveWindow = (widget, track, _tracks, hour) => ({ firstHour: track.branch.firstHour,
      lastHour: widget.time.mode === 'follow' ? hour : track.branch.lastHour });
      export const createWidget = () => { throw Error('Pin is outside this fixture'); };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = (_clock, follow) => { globalThis.__forceNumeric.subscriptions.push(follow); };
      export const usePlaybackSpeed = clock => clock.getSnapshot().hoursPerSecond;`,
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => globalThis.__forceNumeric.name + tag,
      color: () => [50, 100, 150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });`,
    '/charts/useChartTokens.ts': `const tokens = ${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))};
      export const useChartTokens = () => tokens; export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__forceNumeric.pickers.push(props); return null; };',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({ hovered: null, rowProps: () => ({}) });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.open ? props.children : props.main;',
    '/charts/drill/DrillParts.tsx': 'export default props => { globalThis.__forceNumeric.lists.push(props); return null; };',
    '/charts/TimeChart.tsx': `import { createElement } from 'react';
      export default props => { globalThis.__forceNumeric.charts.push(props); return createElement('canvas', { 'data-numeric-chart': true }); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'force-content-refresh', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return source.slice(0, source.indexOf('export function useProducts<T>'))
          .replace('export async function loadProduct<T>(', 'async function unusedLoadProduct<T>(')
          + `export const useProducts = () => ({ data: undefined, retry() {} });
            export const useProduct = (_client, request) => { const s = globalThis.__forceNumeric;
              return { data: !request ? undefined : request.product === 'country-groups' ? s.groups
                : request.params.tags.length === 1 ? s.detail : s.main, error: s.error, retry() {} }; };
            export const loadProduct = (client, request, _signal, _validate, _source, _restart, publish) => {
              const { data, error } = useProduct(client, request); if (error) throw Error(error);
              if (data !== undefined) publish?.(data); return new Promise(() => {}); };
            export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
            export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`;
        // The race's view read answers in the same render, as its products do here.
        if (path.endsWith('/dashboard/viewRead.ts')) return `export const useViewRead = (lane, view, _viewKey, active, read) => { let data, error;
          if (active && lane != null && view != null) try { void read(view, new AbortController().signal, value => { data = value; }); } catch (failure) { error = failure.message; }
          return { data, error, retry() {} }; };`;
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        const module = ['/charts/race/PartsRaceView.tsx', '/charts/race/CountryRaceView.tsx'].find(suffix => path.endsWith(suffix));
        if (module) {
          source = source.replace(/\buseMemo\(/g, `globalThis.__forceMemo(${JSON.stringify(module)}, `);
          if (module.endsWith('/PartsRaceView.tsx')) source = source.replace(/\buseReducer\(/g, 'globalThis.__forceReducer(')
            .replace(/\buseEffect\(/g, `globalThis.__forceEffect(${JSON.stringify(module)}, `);
          else source = source.replace('const tokens = useChartTokens();',
            'globalThis.__forceNumeric.views.push({ mix, presentation, moment, playhead }); const tokens = useChartTokens();');
          return source;
        }
      },
    }] });
  try {
    const Air = (await vite.ssrLoadModule('/ui/src/widgets/air-force-size/AirForceSizeWidget.tsx')).default as ComponentType<WidgetProps>;
    const Navy = (await vite.ssrLoadModule('/ui/src/widgets/navy-size/NavySizeWidget.tsx')).default as ComponentType<WidgetProps>;
    for (const [kind, Widget] of [['air-force-size', Air], ['navy-size', Navy]] as const) {
      for (const grouped of [false, true]) {
        state.memos.clear(); state.drill = undefined; state.error = undefined;
        let hour = 25, groups = structuredClone(group);
        const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], branches: [] } },
          branch: { id: '1', firstHour: 0, lastHour: 100, gaps: [] } };
        const props = { widget: { id: 'widget', kind, configVersion: 1, trackIds: ['track'], countries: grouped
          ? { kind: 'group', group: { type: 'faction', id: '7' }, membership: 'hour', aggregate: false }
          : { kind: 'group', group: { type: 'top', count: 1 }, membership: 'hour', aggregate: false },
          time: { mode: 'follow' }, options: { view: 'lines', endLabels: false } },
          runtime: { tracks: [track], client: {}, clock: { getHour: () => hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } },
          active: true, onDiagnostics: (notes: string[]) => { state.diagnostics = notes; } } as unknown as WidgetProps;
        let mounted: ChartPicks | undefined;
        const render = (main?: PartsRaceReply, detail = main, pendingGroups = main === undefined) => {
          state.main = main; state.detail = detail; state.groups = pendingGroups ? undefined : groups;
          state.charts = []; state.views = []; state.lists = []; state.pickers = []; state.effects = []; state.cursors.clear();
          const html = renderToStaticMarkup(createElement(Widget, props));
          for (const effect of state.effects) effect();
          const chart = state.charts[0];
          if (chart) {
            assert.equal(state.charts.length, 1);
            mounted ??= new ChartPicks();
            mounted.update({ owner: chart.pickOwner, extent: chart.extent, branchStart: chart.branchStart,
              active: chart.active, onPick: chart.onPick });
          } else {
            mounted?.update({ owner: null, extent: [0, 1], branchStart: 0, active: false });
            mounted = undefined; // The mounted TimeChart cancels queued picks on cleanup.
          }
          return { html, chart };
        };
        const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 40], false);
        const context = { frame, until: 25, point: { x: frame.toX(25), y: frame.toY(20) } };
        const first = render(forces);
        assert.ok(first.chart);
        const canvas = mounted!, queued = canvas.drawn(context, first.chart.onPick);
        let renamed = structuredClone(forces);
        for (let tick = 1; tick <= 5; tick++) {
          hour = 80 + tick; track.source.revision = String(tick + 1); state.name = `Later name ${tick}: `;
          renamed = { ...renamed, parts: renamed.parts.map(part => ({ ...part, name: 'Later part name', unresolved: true })), warnings: ['Named refusal'] };
          groups = { ...groups, factions: groups.factions.map(faction => ({ ...faction, name: `Later faction ${tick}`,
            names: faction.names.map(name => ({ ...name, name: `Later faction ${tick}` })) })) };
          const refreshed = render(tick % 2 ? undefined : renamed);
          assert.equal(mounted, canvas, `${kind}: pending content keeps the actual TimeChart mounted`);
          assert.equal(refreshed.chart.pickOwner, first.chart.pickOwner, `${kind}: names, metadata and top-N playback do not replace numeric ownership`);
          if (tick % 2) assert.match(refreshed.html, /Updating/);
          else assert.match(state.diagnostics.join(' '), /Named refusal/);
        }
        assert.equal(canvas.pick(queued, 25), true);
        assert.equal(drillState()?.breakdown, 'OLD', 'deliver the actual drawn actor, rather than the later top country');
        const detail = { ...renamed, series: renamed.series.filter(series => series.tag === 'OLD') };
        const opened = render(renamed, detail);
        assert.ok(opened.chart);
        assert.ok(state.views[0].mix?.compactStack, 'the full drill inventory uses compact stack geometry');
        assert.equal(state.views[0].presentation?.windowedLegend, true);
        assert.deepEqual(state.lists.map(list => [list.virtual, list.active]), [[true, true]]);
        props.widget.time = { mode: 'campaign' }; track.axisOffset = 7; hour = 72;
        render(renamed, detail);
        assert.equal(state.views[0].moment, 79, 'a following drill uses its aligned clock rather than its campaign-mode parent end');
        assert.equal(state.views[0].playhead, 79);
        assert.equal(state.subscriptions.at(-1), true, 'a following drill keeps its hour subscription');
        props.widget.time = { mode: 'follow' }; track.axisOffset = 0; hour = 85;
        render(renamed, detail);
        const detailCanvas = mounted!, detailPick = detailCanvas.drawn(context, opened.chart.onPick);
        const pendingDetail = render(undefined, undefined);
        assert.equal(mounted, detailCanvas);
        assert.equal(pendingDetail.chart.pickOwner, opened.chart.pickOwner);
        assert.equal(drillState()?.breakdown, 'OLD', 'metadata refresh must not close the open drill');
        render(renamed, detail);
        assert.equal(state.views[0].mix?.parts?.[0].label, 'Later part name');
        assert.equal(detailCanvas.pick(detailPick, 25), true);
        assert.equal(drillState()?.hour, 25);
        const beforeChangedDetail = detailCanvas.drawn(context, state.charts[0].onPick);
        const changedDetail = structuredClone(detail); changedDetail.series[0].runs[0].values[0] = 19;
        render(renamed, changedDetail);
        assert.equal(detailCanvas.pick(beforeChangedDetail, 25), false, 'a changed independent drill quantity cancels its pending pick');

        state.drill = undefined;
        const ambiguous = render(renamed), selectorCanvas = mounted!;
        assert.equal(selectorCanvas.pick(selectorCanvas.drawn({ frame, until: 85 }, ambiguous.chart.onPick), 25), true);
        // The chooser retains the native census order used by the drawn lines.
        const actors = ['OLD', 'NEW'];
        assert.deepEqual(drillState()?.choices?.tags, actors);
        render(renamed);
        assert.deepEqual(state.pickers.map(picker => [picker.allowedTags, picker.flagAt?.hour]), [[actors, 25]],
          'ambiguous picks keep only the drawn candidates at the held hour in the searched picker');
        state.drill = undefined;
        const undrawn = render(renamed), undrawnCanvas = mounted!;
        assert.equal(undrawnCanvas.pick(undrawnCanvas.drawn({ frame, until: 25 }, undrawn.chart.onPick), 90), true);
        assert.equal(drillState()?.breakdown, null, 'an hour beyond the witnessed draw cannot open a later actor');
        const changes = ['quantity', 'part', 'reference', 'identity', 'selection', 'membership', 'view', 'extent', 'client', 'source', 'branch', 'inactive'] as const;
        for (const change of changes) {
          const before = render(renamed), current: ChartPicks = mounted!, drawn = current.drawn(context, before.chart.onPick);
          const changed = structuredClone(renamed);
          if (change === 'quantity') changed.series[0].runs[0].total = 21;
          if (change === 'part') changed.parts[0].key = 'different native part';
          if (change === 'reference') changed.series[0].runs[0].referenceTotal = 41;
          if (change === 'identity') { changed.series[0].segments = [{ firstHour: 60, lastHour: 100 }];
            changed.series[0].runs = [{ firstHour: 60, lastHour: 100, startsSegment: true, total: 10, values: [10], referenceTotal: 40 }]; }
          if (change === 'selection') props.widget.options = { ...props.widget.options, ownership: kind === 'air-force-size' ? 'effective' : 'logical' };
          if (change === 'membership') { if (!grouped) continue;
            groups = { ...groups, factions: groups.factions.map(faction => ({ ...faction,
              members: faction.members.map(member => ({ ...member, lastHour: 90 })) })) }; }
          if (change === 'view') props.widget.options = { ...props.widget.options, view: 'mix' };
          if (change === 'extent') track.branch.lastHour++;
          if (change === 'client') props.runtime.client = { ...props.runtime.client };
          if (change === 'source') track.source.id += '-changed';
          if (change === 'branch') track.branch.id += '-changed';
          if (change === 'inactive') props.active = false;
          render(changed);
          if (change === 'view') assert.equal(state.views[0].mix?.compactStack, true, 'the main full inventory uses compact geometry');
          assert.equal(current.pick(drawn, 25), false, `${kind}: changed ${change} cannot deliver an obsolete or reused-tag actor`);
        }
        props.active = true; state.drill = undefined; props.widget.options = { ...props.widget.options, view: 'lines' };
        const last = render(renamed), lastCanvas = mounted!, pending = lastCanvas.drawn(context, last.chart.onPick);
        const lastSource = track.source.id;
        track.source.id = 'other source'; assert.equal(render().chart, undefined);
        track.source.id = lastSource; assert.equal(render().chart, undefined, 'A → B → A cannot recover a previously retained plot');
        assert.equal(lastCanvas.pick(pending, 25), false);
        state.error = 'Content failed'; assert.match(render().html, /Content failed/); state.error = undefined;
      }
    }
  } finally {
    await vite.close(); delete host.__forceNumeric; delete host.__forceMemo; delete host.__forceEffect; delete host.__forceReducer;
  }
});
