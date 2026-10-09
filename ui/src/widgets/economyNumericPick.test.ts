import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../charts/frame.ts';
import { ChartPicks } from '../charts/pick.ts';
import type { TimeChartProps } from '../charts/TimeChart';
import type { CountryRaceViewProps } from '../charts/race/CountryRaceView';
import type { WidgetProps } from '../dashboard/definitions';
import { setToken } from '../dashboard/options.ts';
import type { CountryGroupReply, FuelLabel, FuelReply, ResourceBalanceReply } from '../generated/dashboard';
import { validateFuelReply } from './fuel/fuelData.ts';

const fuelLabel = (key: string): FuelLabel => ({ key, template: `${key}: {NUM}`, arguments: ['NUM'], reason: null });
const fuel: FuelReply = { branchId: '1', tag: 'ONE', quantityScale: 32768, gainScale: 100000, dayHours: 24, detailHours: 720,
  gainLabels: [fuelLabel('Base'), fuelLabel('States'), fuelLabel('Oil'), fuelLabel('Flat gain')],
  transferLabels: [fuelLabel('Incoming'), fuelLabel('Outgoing'), fuelLabel('Flat cost')],
  consumerNames: ['Ground', 'Air', 'Sea'], warnings: [], reads: [{ firstHour: 0, lastHour: 100, startsSegment: true,
    country: '7', countryInstance: null, continuityStartHour: null, awardsReason: 'Applied fuel updates were not recorded',
    fuelRaw: String(80 * 32768), capacityRaw: String(100 * 32768), lendLeaseRaw: '0', leftOut: null,
    day: { gainedRaw: '500000', burnedRaw: String(10 * 32768), receivedRaw: ['1000000', '0', '0'], otherRaw: '0', awards: null } }] };
const resource: ResourceBalanceReply = { branchId: '1', tag: 'ONE', decimals: 5, warnings: [],
  resources: [{ id: 5, token: 'r', ordinal: 1, name: 'First resource name', icon: null }],
  series: [{ id: 5, runs: [{ firstHour: 0, lastHour: 100, startsSegment: true, producedRaw: '8000000',
    importedRaw: '0', subjectsRaw: '0', toExportRaw: '0', exportedRaw: '0', availableRaw: '8000000',
    afterProjectsRaw: '8000000', productionRaw: '-2000000' }] }], dealHours: [{ firstHour: 0, lastHour: 100 }], trades: [] };
const group: CountryGroupReply = { branchId: '1', humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
  countryIdentities: [{ tag: 'ONE', identities: [{ firstHour: 0, lastHour: 100, slot: 1, countryId: 7 }] }],
  unobserved: { humans: [], factions: [], wars: [] }, factions: [{ id: '7', firstHour: 0, lastHour: 100, recordedName: '',
    name: 'First faction name', nameReason: null, names: [{ firstHour: 0, lastHour: 100, name: 'First faction name', reason: null }],
    members: [{ tag: 'ONE', firstHour: 0, lastHour: 100 }], leaders: [{ tag: 'ONE', firstHour: 0, lastHour: 100 }], incomplete: [] }] };

test('economy and shared race picks keep their mounted numeric owner through content and playback refresh', async () => {
  assert.equal(validateFuelReply(fuel), fuel);
  const state = { product: {} as { data?: unknown; error?: string; retry: () => void }, groups: {} as { data?: CountryGroupReply; retry: () => void }, charts: [] as TimeChartProps[],
    effects: [] as (() => unknown)[], memos: new Map<string, { deps: readonly unknown[]; value: unknown }[]>(),
    cursors: new Map<string, number>(), diagnostics: [] as string[], name: '' };
  const host = globalThis as typeof globalThis & { __economyNumeric?: typeof state;
    __economyMemo?: (id: string, factory: () => unknown, deps: readonly unknown[]) => unknown };
  host.__economyNumeric = state;
  // SSR has no mounted hooks. Retain only the actual useMemo slots between renders;
  // the widget's loading/error branches and the shared views still render unchanged.
  host.__economyMemo = (id, factory, deps) => {
    const index = state.cursors.get(id) ?? 0;
    state.cursors.set(id, index + 1);
    let slots = state.memos.get(id);
    if (!slots) { slots = []; state.memos.set(id, slots); }
    const before = slots[index];
    if (before && before.deps.length === deps.length && before.deps.every((value, i) => Object.is(value, deps[i]))) return before.value;
    const value = factory(); slots[index] = { deps: [...deps], value }; return value;
  };
  const memoModules = ['/widgets/fuel/FuelWidget.tsx', '/widgets/resource-balance/ResourceBalanceWidget.tsx',
    '/widgets/resource-balance/useResourceBalance.ts', '/charts/race/CountryRaceView.tsx', '/charts/drill/BreakdownChart.tsx',
    '/dashboard/comparisonCountries.ts', '/dashboard/CountryComparisons.tsx'];
  const replacements: Record<string, string> = {
    '/dashboard/model.ts': `export const resolveWindow = (widget, track) => widget.time.mode === 'pin'
      ? { firstHour: track.branch.firstHour + widget.time.windows[track.id].firstOffset,
        lastHour: track.branch.firstHour + widget.time.windows[track.id].lastOffset }
      : { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour };
      export const createWidget = () => { throw Error('Pin is outside this fixture'); };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = () => {};
      export const usePlaybackSpeed = clock => clock.getSnapshot().hoursPerSecond;`,
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => globalThis.__economyNumeric.name + tag,
      color: () => [50, 100, 150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });`,
    '/charts/useChartTokens.ts': `const tokens = { categories: ['#123456', '#654321'], accent: '#00aaff', muted: '#888888',
      surface: '#000000', positive: '#00ff00', error: '#ff0000', countryColours: 'game', fontSize: { body: 12 } };
      export const useChartTokens = () => tokens; export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({ hovered: null, rowProps: () => ({}) });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.open ? props.children : props.main;',
    '/charts/TimeChart.tsx': `import { createElement } from 'react';
      export default props => { globalThis.__economyNumeric.charts.push(props); return createElement('canvas', { 'data-numeric-chart': true }); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'economy-content-refresh', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return source.slice(0, source.indexOf('export function useProducts<T>'))
          + `const idle = { retry() {} };
            function check(data, request, validate) {
              if (data === undefined) return;
              validate(data);
              if (data.branchId !== request.params.branchId || request.product !== 'country-groups' && data.tag !== request.params.tag)
                throw Error('The economy fixture reply belongs to another requested country or branch.');
            }
            export function useProducts(_client, requests, active, validate) {
              if (!active || !requests.length) return idle;
              if (requests.some(request => request.product !== 'country-fuel' && request.product !== 'resource-balance')) throw Error('Unexpected economy comparison product.');
              const state = globalThis.__economyNumeric.product;
              const data = requests[0].product === 'resource-balance' && state.data !== undefined ? [state.data] : state.data;
              if (data !== undefined) {
                if (!Array.isArray(data) || data.length !== requests.length) throw Error('The economy comparison fixture has the wrong reply count.');
                data.forEach((reply, index) => check(reply, requests[index], validate));
              }
              return { ...state, data };
            }
            export function useProduct(_client, request, active, validate) {
              if (!request || !active) return idle;
              const state = request.product === 'country-groups' ? globalThis.__economyNumeric.groups
                : request.product === 'resource-balance' ? globalThis.__economyNumeric.product : null;
              if (!state) throw Error('Unexpected economy product: ' + request.product);
              check(state.data, request, validate);
              return state;
            }`;
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        const module = memoModules.find(suffix => path.endsWith(suffix));
        if (module) {
          source = source.replace(/\buseMemo\(/g, `globalThis.__economyMemo(${JSON.stringify(module)}, `);
          // Capture a panel's diagnostics directly; SSR does not rerender its parent's notes state.
          if (module.endsWith('CountryComparisons.tsx')) source = source.replace('onDiagnostics={handlers.get(tag)!}', 'onDiagnostics={props.onDiagnostics}');
          if (module.endsWith('Widget.tsx')) source = source.replace(/\buseEffect\(/g,
            '((effect) => globalThis.__economyNumeric.effects.push(effect))(');
          return source;
        }
      },
    }] });
  try {
    const Fuel = (await vite.ssrLoadModule('/ui/src/widgets/fuel/FuelWidget.tsx')).default as ComponentType<WidgetProps>;
    const Resource = (await vite.ssrLoadModule('/ui/src/widgets/resource-balance/ResourceBalanceWidget.tsx')).default as ComponentType<WidgetProps>;
    const cases: [string, ComponentType<WidgetProps>, FuelReply[] | ResourceBalanceReply][] = [['fuel', Fuel, [fuel]], ['resource-balance', Resource, resource]];
    for (const [kind, Widget, initial] of cases) {
      for (const [detail, grouped] of [[false, false], [true, false], [false, true], [true, true]]) {
        state.memos.clear();
        let groupReply = group;
        const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [], branches: [] } },
          branch: { id: '1', firstHour: 0, lastHour: 100, gaps: [] } };
        const props = { widget: { id: 'widget', kind, configVersion: 1, trackIds: ['track'], countries: grouped
          ? { kind: 'group', group: { type: 'faction', id: '7' }, membership: 'hour', aggregate: false } : { kind: 'tags', tags: ['ONE'] },
          time: detail && kind === 'resource-balance' ? { mode: 'pin', windows: { track: { firstOffset: 50, lastOffset: 50 } } }
            : { mode: 'campaign' }, options: { view: 'time', capacity: false, strip: false, endLabels: false,
            show: detail ? kind === 'fuel' ? 'burn' : 'resource' : '', partResource: setToken(JSON.stringify(['r', 1])) } },
          runtime: { tracks: [track], client: {}, clock: { getHour: () => 100, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } },
          active: true, onDiagnostics: (notes: string[]) => { state.diagnostics = notes; } } as unknown as WidgetProps;
        let mounted: ChartPicks | undefined;
        const render = (data?: unknown, error?: string) => {
          state.product = { data, error, retry() {} }; state.groups = { data: data === undefined ? undefined : groupReply, retry() {} };
          state.charts = []; state.effects = []; state.cursors.clear();
          const html = renderToStaticMarkup(createElement(Widget, props));
          for (const effect of state.effects) effect();
          const chart = state.charts[0];
          if (chart) {
            assert.equal(state.charts.length, 1);
            mounted ??= new ChartPicks();
            mounted.update({ owner: chart.pickOwner, extent: chart.extent, branchStart: chart.branchStart,
              active: chart.active, onPick: chart.onPick });
          } else mounted = undefined; // TimeChart's cleanup cancels queued picks on unmount.
          return { html, chart };
        };
        const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
        const first = render(initial);
        assert.ok(first.chart, `${kind} ${detail}: initial plot`);
        const canvas = mounted!;
        let delivered = 0;
        const callback = first.chart.onPick!;
        const queued = canvas.drawn({ frame, until: null, point: { x: frame.toX(50), y: frame.toY(80) } },
          (hour, context) => { delivered++; callback(hour, context); });
        const pending = render();
        assert.equal(mounted, canvas, `${kind} ${detail}: content pending must keep TimeChart mounted`);
        assert.equal(pending.chart.pickOwner, first.chart.pickOwner);
        assert.match(state.diagnostics.join(' '), /updating from the recorded content/);
        assert.doesNotMatch(pending.html, /First resource name|Ground/);
        const labels = structuredClone(initial);
        groupReply = { ...group, factions: group.factions.map(faction => ({ ...faction, name: 'Other faction name',
          names: faction.names.map(name => ({ ...name, name: 'Other faction name' })) })) };
        if (Array.isArray(labels)) { labels[0].consumerNames = ['Other ground name', 'Other air name', 'Other sea name']; labels[0].warnings = ['Content refusal']; }
        else { labels.resources[0].name = 'Other resource name'; labels.resources[0].icon = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMIWPDhPwAGBALgx3AGRgAAAABJRU5ErkJggg=='; labels.warnings = ['Content refusal']; }
        const renamed = render(labels);
        assert.equal(mounted, canvas);
        assert.equal(renamed.chart.pickOwner, first.chart.pickOwner);
        assert.match(state.diagnostics.join(' '), /Content refusal/);
        assert.equal(canvas.pick(queued, 50), true);
        assert.equal(delivered, 1);
        const changed = structuredClone(labels);
        if (Array.isArray(changed)) changed[0].reads[0].fuelRaw = String(81 * 32768);
        else changed.series[0].runs[0].producedRaw = '8100000';
        assert.notEqual(render(changed).chart.pickOwner, first.chart.pickOwner);
        assert.equal(canvas.pick(queued, 50), false);
        const current = state.charts[0];
        const beforeExtent = canvas.drawn({ frame, until: null }, current.onPick);
        track.branch.lastHour = 101;
        render(changed);
        assert.equal(canvas.pick(beforeExtent, 50), false);
        if (grouped) {
          const beforeMembership = canvas.drawn({ frame, until: null }, state.charts[0].onPick);
          groupReply = { ...groupReply, factions: groupReply.factions.map(faction => ({ ...faction,
            members: faction.members.map(member => ({ ...member, lastHour: 75 })) })) };
          render(changed);
          assert.equal(canvas.pick(beforeMembership, 50), false);
        }
        assert.match(render(undefined, 'Content failed').html, /Content failed/);
        assert.equal(mounted, undefined);
        track.source.id = 'other source';
        assert.equal(render().chart, undefined);
        track.source.id = 'source';
        assert.equal(render().chart, undefined, 'A → B → A cannot resurrect the old numeric plot');
      }
    }
    const Race = (await vite.ssrLoadModule('/ui/src/charts/race/CountryRaceView.tsx')).default as ComponentType<CountryRaceViewProps>;
    state.memos.clear();
    const track = { id: 'track', source: { id: 'source', revision: '1', metadata: { countries: [] } },
      branch: { id: 'branch', firstHour: 0, lastHour: 100, gaps: [] } };
    const native = { recording: 'source', branch: 'branch', selection: 'all', nativeHistory: 'same' };
    const oldLines = [{ tag: 'OLD', id: 'OLD-total', runs: [{ firstHour: 0, lastHour: 100, value: 80 }] }];
    const newLines = [{ tag: 'NEW', id: 'NEW-total', runs: [{ firstHour: 0, lastHour: 100, value: 20 }] }];
    const picked: (string | undefined)[] = [];
    const props = { runtime: { client: {}, clock: { getSnapshot: () => ({ hoursPerSecond: 24 }) } }, track, active: true,
      options: { playback: 'draw-in', endLabels: false }, view: 'lines', extent: [0, 100], playhead: 80, moment: 80, slots: 1,
      ranked: [], lines: oldLines, countryTags: ['OLD'], label: 'Country total', empty: '', formatValue: String,
      tooltip: () => null, pickOwner: native, onPick: (_hour: number, tag?: string) => picked.push(tag) } as unknown as CountryRaceViewProps;
    const canvas = new ChartPicks();
    const render = () => {
      state.charts = []; state.cursors.clear();
      renderToStaticMarkup(createElement(Race, props));
      const chart = state.charts[0];
      assert.ok(chart);
      canvas.update({ owner: chart.pickOwner, extent: chart.extent, branchStart: chart.branchStart, active: chart.active, onPick: chart.onPick });
      return chart;
    };
    const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
    const initial = render();
    assert.equal(initial.pickOwner, native, 'the explicit caller owner is forwarded unchanged');
    const queued = canvas.drawn({ frame, until: 80, point: { x: frame.toX(50), y: frame.toY(80) } }, initial.onPick);
    // Five 10 Hz renders arrive during the existing 500 ms single-click delay.
    for (let tick = 1; tick <= 5; tick++) {
      props.playhead = props.moment = 80 + tick;
      track.source.revision = String(tick + 1); state.name = `Name at ${props.moment}: `;
      props.lines = newLines; props.countryTags = ['NEW']; props.label = `Renamed ${tick}`;
      assert.equal(render().pickOwner, native);
    }
    assert.equal(canvas.pick(queued, 50), true);
    assert.deepEqual(picked, ['OLD'], 'the click keeps its witnessed actor, rather than the latest drawn top-N actor');
    for (const change of ['owner', 'source', 'branch', 'view', 'extent'] as const) {
      const before = render(), drawn = canvas.drawn({ frame, until: null }, before.onPick);
      if (change === 'source') track.source.id += '-changed';
      if (change === 'branch') track.branch.id += '-changed';
      if (change === 'view') { props.view = 'mix'; props.mix = { tag: 'NEW', bands: [newLines[0].runs] }; }
      if (change === 'extent') props.extent = [0, 101];
      else props.pickOwner = { ...native, source: track.source.id, branch: track.branch.id, view: props.view, change };
      render();
      assert.equal(canvas.pick(drawn, 50), false, change);
    }
    props.pickOwner = native;
    const a = render(), fromA = canvas.drawn({ frame, until: null }, a.onPick);
    props.pickOwner = {}; render(); props.pickOwner = native; render();
    assert.equal(canvas.pick(fromA, 50), false, 'an explicit owner A → B → A stays obsolete');
    props.pickOwner = undefined; props.view = 'lines'; props.lines = oldLines; props.extent = [0, 100];
    const fallback = render(), fallbackPick = canvas.drawn({ frame, until: null }, fallback.onPick);
    track.source.revision = 'metadata only'; props.countryTags = ['LATER']; props.moment++;
    props.lines = oldLines.map(line => ({ ...line, color: '#abcdef', endLabel: (_hour: number, name: string) => `Renamed ${name}` }));
    state.name = 'Other displayed name: ';
    assert.equal(render().pickOwner, fallback.pickOwner, 'new line/style wrappers preserve the same IDs and numeric run references');
    assert.equal(canvas.pick(fallbackPick, 50), true);
    assert.equal(picked.at(-1), 'OLD');
    props.view = 'mix'; props.mix = { tag: 'OLD', lines: oldLines };
    const mixFallback = render(), mixPick = canvas.drawn({ frame, until: null }, mixFallback.onPick);
    props.mix = { ...props.mix, lines: oldLines.map(line => ({ ...line, color: '#fedcba', endLabel: () => 'Other label' })) };
    assert.equal(render().pickOwner, mixFallback.pickOwner, 'mix-line wrapper replacement also preserves numeric ownership');
    assert.equal(canvas.pick(mixPick, 50), true);
    assert.equal(picked.at(-1), 'OLD');
    props.view = 'lines';
    for (const change of ['client', 'source', 'branch', 'id', 'runs', 'extent', 'view', 'model'] as const) {
      const before = render(), drawn = canvas.drawn({ frame, until: null }, before.onPick);
      if (change === 'client') props.runtime.client = { ...props.runtime.client };
      if (change === 'source') track.source.id += '-other';
      if (change === 'branch') track.branch.id += '-other';
      if (change === 'id') props.lines = props.lines.map(line => ({ ...line, id: `${line.id}-other` }));
      if (change === 'runs') props.lines = props.lines.map(line => ({ ...line, runs: [...line.runs] }));
      if (change === 'extent') props.extent = [0, 102];
      if (change === 'view') props.view = 'mix';
      if (change === 'model') props.mix = { tag: 'NEW', bands: [[{ firstHour: 0, lastHour: 100, value: 21 }]] };
      render();
      assert.equal(canvas.pick(drawn, 50), false, `fallback ${change}`);
    }
    const bandSource = props.mix!;
    const bandA = render(), bandPick = canvas.drawn({ frame, until: null }, bandA.onPick);
    props.mix = { tag: 'NEW', bands: [[{ firstHour: 0, lastHour: 100, value: 22 }]] }; render();
    props.mix = bandSource; render();
    assert.equal(canvas.pick(bandPick, 50), false, 'fallback A → B → A remains obsolete');
  } finally {
    await vite.close(); delete host.__economyNumeric; delete host.__economyMemo;
  }
});
